//! The write-ahead log (WAL).
//!
//! Before a write touches the memtable, it is appended to this file. If the
//! process crashes, the memtable is lost but the log is not, so on restart we
//! replay the log and end up exactly where we were.
//!
//! ## Record format (all integers little-endian)
//!
//! ```text
//! +---------+--------+-------------+-------------+-----------+-------------+
//! | crc u32 | kind u8| key_len u32 | val_len u32 | key bytes | value bytes |
//! +---------+--------+-------------+-------------+-----------+-------------+
//!            \__________________ covered by the crc ___________________/
//! ```
//!
//! `kind` is 1 for a put and 2 for a delete (deletes have `val_len = 0`).
//!
//! ## Torn writes
//!
//! A crash can stop a write half way, leaving a partial record at the end of
//! the file. On replay we read records until one is incomplete or fails its
//! checksum, keep everything before it, and cut the file there. A write that
//! was never fully on disk is treated as never having happened.

use crate::crc32::Crc32;
use crate::error::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const HEADER_LEN: usize = 4 + 1 + 4 + 4;
const KIND_PUT: u8 = 1;
const KIND_DELETE: u8 = 2;

/// One logged operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// What happened while replaying the log on start-up.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Replay {
    pub records: Vec<Record>,
    /// Bytes at the end of the file that were dropped because they formed an
    /// incomplete or corrupt record.
    pub dropped_bytes: u64,
}

pub struct Wal {
    file: File,
    /// If true, every append waits until the data is physically on disk
    /// (`fsync`). Safer, but much slower.
    sync: bool,
}

impl Wal {
    /// Open (or create) the log at `path`, replay what is in it, and return
    /// the log ready for appending together with the replayed records.
    pub fn open(path: &Path, sync: bool) -> Result<(Wal, Replay)> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        let (records, good_len) = parse(&bytes);
        let dropped_bytes = (bytes.len() - good_len) as u64;
        if dropped_bytes > 0 {
            // Cut off the torn tail so new records follow the last good one.
            file.set_len(good_len as u64)?;
            file.sync_all()?;
        }

        Ok((
            Wal { file, sync },
            Replay {
                records,
                dropped_bytes,
            },
        ))
    }

    pub fn append(&mut self, record: &Record) -> Result<()> {
        let buf = encode(record)?;
        // One `write_all` per record keeps a record's bytes together in the file.
        self.file.write_all(&buf)?;
        if self.sync {
            self.file.sync_data()?;
        }
        Ok(())
    }

    /// Force everything written so far onto disk.
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }
}

/// Turn a record into bytes in the format described at the top of the file.
pub fn encode(record: &Record) -> Result<Vec<u8>> {
    let (kind, key, value): (u8, &[u8], &[u8]) = match record {
        Record::Put { key, value } => (KIND_PUT, key, value),
        Record::Delete { key } => (KIND_DELETE, key, &[]),
    };
    let key_len = len_u32(key.len(), "key")?;
    let val_len = len_u32(value.len(), "value")?;

    let mut buf = Vec::with_capacity(HEADER_LEN + key.len() + value.len());
    buf.extend_from_slice(&[0; 4]); // checksum placeholder, filled in below
    buf.push(kind);
    buf.extend_from_slice(&key_len.to_le_bytes());
    buf.extend_from_slice(&val_len.to_le_bytes());
    buf.extend_from_slice(key);
    buf.extend_from_slice(value);

    let mut crc = Crc32::new();
    crc.update(&buf[4..]);
    buf[..4].copy_from_slice(&crc.finish().to_le_bytes());
    Ok(buf)
}

fn len_u32(len: usize, what: &str) -> Result<u32> {
    u32::try_from(len).map_err(|_| Error::Corrupt(format!("{what} is too large ({len} bytes)")))
}

/// Parse as many complete, valid records as possible.
/// Returns the records and how many bytes they took up.
fn parse(bytes: &[u8]) -> (Vec<Record>, usize) {
    let mut records = Vec::new();
    let mut pos = 0;
    // `while let` keeps looping as long as the pattern matches.
    while let Some((record, len)) = parse_one(&bytes[pos..]) {
        records.push(record);
        pos += len;
    }
    (records, pos)
}

/// Parse one record from the start of `buf`.
/// `None` means "incomplete or corrupt": the caller stops there.
fn parse_one(buf: &[u8]) -> Option<(Record, usize)> {
    if buf.len() < HEADER_LEN {
        return None;
    }
    let crc = u32::from_le_bytes(buf[0..4].try_into().ok()?);
    let kind = buf[4];
    let key_len = u32::from_le_bytes(buf[5..9].try_into().ok()?) as usize;
    let val_len = u32::from_le_bytes(buf[9..13].try_into().ok()?) as usize;

    // `checked_add` returns None instead of overflowing on garbage lengths.
    let total = HEADER_LEN.checked_add(key_len)?.checked_add(val_len)?;
    if buf.len() < total {
        return None;
    }

    let mut check = Crc32::new();
    check.update(&buf[4..total]);
    if check.finish() != crc {
        return None;
    }

    let key = buf[HEADER_LEN..HEADER_LEN + key_len].to_vec();
    let record = match kind {
        KIND_PUT => Record::Put {
            key,
            value: buf[HEADER_LEN + key_len..total].to_vec(),
        },
        KIND_DELETE if val_len == 0 => Record::Delete { key },
        _ => return None,
    };
    Some((record, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(k: &str, v: &str) -> Record {
        Record::Put {
            key: k.into(),
            value: v.into(),
        }
    }

    #[test]
    fn encode_then_parse_round_trips() {
        let recs = vec![
            put("a", "1"),
            Record::Delete { key: b"a".to_vec() },
            put("", ""),
        ];
        let mut bytes = Vec::new();
        for r in &recs {
            bytes.extend(encode(r).unwrap());
        }
        let (parsed, len) = parse(&bytes);
        assert_eq!(parsed, recs);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn stops_at_torn_tail() {
        let mut bytes = encode(&put("k1", "v1")).unwrap();
        let good = bytes.len();
        let second = encode(&put("k2", "v2")).unwrap();
        bytes.extend_from_slice(&second[..second.len() - 1]); // lose the last byte
        let (parsed, len) = parse(&bytes);
        assert_eq!(parsed, vec![put("k1", "v1")]);
        assert_eq!(len, good);
    }

    #[test]
    fn stops_at_bad_checksum() {
        let mut bytes = encode(&put("k1", "v1")).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF; // flip bits in the value
        let (parsed, len) = parse(&bytes);
        assert!(parsed.is_empty());
        assert_eq!(len, 0);
    }

    #[test]
    fn huge_length_in_garbage_does_not_panic() {
        let mut bytes = vec![0u8; HEADER_LEN];
        bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse(&bytes).0, vec![]);
    }
}
