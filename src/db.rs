//! The public face of the engine: open a database, put, get, delete, scan.

use crate::error::Result;
use crate::memtable::{Entry, Lookup, MemTable};
use crate::wal::{Record, Wal};
use std::fs;
use std::ops::Bound;
use std::path::{Path, PathBuf};

const WAL_FILE: &str = "wal.log";

/// Settings for [`Db::open_with`].
#[derive(Debug, Clone)]
pub struct Options {
    /// `fsync` after every write. On by default: a write that returned `Ok`
    /// survives even a power cut. Turn it off for speed in tests or benchmarks;
    /// then only a process crash (not a power cut) is safe.
    pub sync_writes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { sync_writes: true }
    }
}

pub struct Db {
    dir: PathBuf,
    wal: Wal,
    memtable: MemTable,
    /// Bytes thrown away from a torn log tail when the database was opened.
    recovered_dropped_bytes: u64,
}

impl Db {
    /// Open the database stored in `dir`, creating it if needed.
    ///
    /// `impl AsRef<Path>` means callers can pass a `&str`, `String`,
    /// `PathBuf` or `&Path`: anything that can be viewed as a path.
    pub fn open(dir: impl AsRef<Path>) -> Result<Db> {
        Self::open_with(dir, Options::default())
    }

    pub fn open_with(dir: impl AsRef<Path>, options: Options) -> Result<Db> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        let (wal, replay) = Wal::open(&dir.join(WAL_FILE), options.sync_writes)?;
        let mut memtable = MemTable::new();
        // Replaying the log, in order, rebuilds the memtable exactly.
        for record in replay.records {
            match record {
                Record::Put { key, value } => memtable.put(&key, &value),
                Record::Delete { key } => memtable.delete(&key),
            }
        }

        Ok(Db {
            dir,
            wal,
            memtable,
            recovered_dropped_bytes: replay.dropped_bytes,
        })
    }

    /// Store `value` under `key`, replacing any previous value.
    ///
    /// Takes `&mut self`: Rust's borrow rules guarantee nobody else is reading
    /// the database while we change it, with no locks needed (yet).
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        // Log first, then apply. If the log write fails we return the error
        // and the memtable is untouched, so memory and disk never disagree.
        self.wal.append(&Record::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        })?;
        self.memtable.put(key, value);
        Ok(())
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<()> {
        self.wal.append(&Record::Delete { key: key.to_vec() })?;
        self.memtable.delete(key);
        Ok(())
    }

    /// Look a key up. Returns an owned copy so the caller can keep it after
    /// the database changes.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        match self.memtable.get(key) {
            Lookup::Found(v) => Some(v.to_vec()),
            // Milestone 2: on `Missing`, search the SSTables on disk too.
            Lookup::Deleted | Lookup::Missing => None,
        }
    }

    /// All live key/value pairs with `start <= key < end`, in key order.
    /// `None` means "unbounded" on that side.
    pub fn scan(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Vec<(Vec<u8>, Vec<u8>)> {
        let lo = start.map_or(Bound::Unbounded, |s| Bound::Included(s.to_vec()));
        let hi = end.map_or(Bound::Unbounded, |e| Bound::Excluded(e.to_vec()));
        // BTreeMap::range panics on a backwards range, so answer those directly.
        // (`if let ... && ...` is a "let chain", new in the 2024 edition.)
        if let (Bound::Included(a), Bound::Excluded(b)) = (&lo, &hi)
            && a >= b
        {
            return Vec::new();
        }
        self.memtable
            .range((lo, hi))
            // `filter_map` keeps values and skips tombstones in one step.
            .filter_map(|(k, e)| match e {
                Entry::Value(v) => Some((k.to_vec(), v.clone())),
                Entry::Tombstone => None,
            })
            .collect()
    }

    /// Flush buffered log writes to disk (useful when `sync_writes` is off).
    pub fn sync(&mut self) -> Result<()> {
        self.wal.sync()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Number of keys in the memtable, tombstones included.
    pub fn memtable_len(&self) -> usize {
        self.memtable.len()
    }

    pub fn memtable_bytes(&self) -> usize {
        self.memtable.approx_size()
    }

    pub fn recovered_dropped_bytes(&self) -> u64 {
        self.recovered_dropped_bytes
    }
}
