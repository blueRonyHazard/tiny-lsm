//! The memtable: recent writes, kept sorted in memory.
//!
//! We use `BTreeMap` from the standard library. It keeps keys in order, which
//! matters later: when the memtable gets big we will write it to disk as a
//! *sorted* file (an SSTable) just by iterating over it.

use std::collections::BTreeMap;
use std::ops::RangeBounds;

/// What the memtable knows about a key.
///
/// A delete cannot simply remove the key: in later milestones an older value
/// may still live in a file on disk, and removing the key here would make that
/// old value visible again. So a delete is stored as a **tombstone**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Value(Vec<u8>),
    Tombstone,
}

/// Result of looking a key up in one layer of the engine.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// The key has this value.
    Found(&'a [u8]),
    /// The key was deleted here; stop searching older layers.
    Deleted,
    /// This layer knows nothing about the key; keep searching.
    Missing,
}

#[derive(Debug, Default)]
pub struct MemTable {
    map: BTreeMap<Vec<u8>, Entry>,
    /// Rough number of bytes held, used later to decide when to flush.
    approx_size: usize,
}

impl MemTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) {
        self.insert(key, Entry::Value(value.to_vec()));
    }

    pub fn delete(&mut self, key: &[u8]) {
        self.insert(key, Entry::Tombstone);
    }

    fn insert(&mut self, key: &[u8], entry: Entry) {
        let added = key.len() + entry_len(&entry);
        // `insert` hands back the previous entry, if any, so we can keep the
        // size estimate accurate when a key is overwritten.
        match self.map.insert(key.to_vec(), entry) {
            Some(old) => {
                self.approx_size = self.approx_size + added - key.len() - entry_len(&old);
            }
            None => self.approx_size += added,
        }
    }

    /// Note the lifetime `'_`: the returned slice borrows from the memtable,
    /// so the compiler will not let the memtable change while you hold it.
    pub fn get(&self, key: &[u8]) -> Lookup<'_> {
        match self.map.get(key) {
            Some(Entry::Value(v)) => Lookup::Found(v),
            Some(Entry::Tombstone) => Lookup::Deleted,
            None => Lookup::Missing,
        }
    }

    /// Iterate over a key range in sorted order, tombstones included.
    pub fn range<R>(&self, range: R) -> impl Iterator<Item = (&[u8], &Entry)>
    where
        R: RangeBounds<Vec<u8>>,
    {
        self.map.range(range).map(|(k, e)| (k.as_slice(), e))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn approx_size(&self) -> usize {
        self.approx_size
    }
}

fn entry_len(e: &Entry) -> usize {
    match e {
        Entry::Value(v) => v.len(),
        Entry::Tombstone => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_overwrite() {
        let mut m = MemTable::new();
        m.put(b"a", b"1");
        m.put(b"a", b"22");
        assert_eq!(m.get(b"a"), Lookup::Found(b"22"));
        assert_eq!(m.get(b"b"), Lookup::Missing);
        assert_eq!(m.len(), 1);
        assert_eq!(m.approx_size(), 3);
    }

    #[test]
    fn delete_leaves_tombstone() {
        let mut m = MemTable::new();
        m.put(b"a", b"1");
        m.delete(b"a");
        assert_eq!(m.get(b"a"), Lookup::Deleted);
        assert_eq!(m.approx_size(), 1);
    }

    #[test]
    fn range_is_sorted() {
        let mut m = MemTable::new();
        for k in ["c", "a", "b"] {
            m.put(k.as_bytes(), b"x");
        }
        let keys: Vec<_> = m.range(..).map(|(k, _)| k.to_vec()).collect();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }
}
