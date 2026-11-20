//! End-to-end tests: write, "crash" (drop the Db), reopen, check.
//!
//! Files in `tests/` are compiled as separate programs that can only use the
//! crate's public API, just like a real user would.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use tiny_lsm::{Db, Options};

/// A temporary directory that deletes itself when it goes out of scope.
/// This is Rust's RAII pattern: cleanup lives in `Drop`, so it runs even if
/// the test panics half way.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tiny-lsm-test-{}-{}-{}",
            name,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fast() -> Options {
    Options { sync_writes: false }
}

#[test]
fn data_survives_reopen() {
    let dir = TempDir::new("reopen");
    {
        let mut db = Db::open_with(&dir.0, fast()).unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"b", b"2").unwrap();
        db.put(b"a", b"3").unwrap();
        db.delete(b"b").unwrap();
    } // `db` is dropped here: our simulated crash

    let db = Db::open(&dir.0).unwrap();
    assert_eq!(db.get(b"a"), Some(b"3".to_vec()));
    assert_eq!(db.get(b"b"), None);
    assert_eq!(db.recovered_dropped_bytes(), 0);
}

#[test]
fn torn_write_is_dropped_and_log_keeps_working() {
    let dir = TempDir::new("torn");
    {
        let mut db = Db::open_with(&dir.0, fast()).unwrap();
        db.put(b"good", b"yes").unwrap();
    }
    // Simulate a crash in the middle of writing the next record.
    let mut f = OpenOptions::new()
        .append(true)
        .open(dir.0.join("wal.log"))
        .unwrap();
    f.write_all(&[0xAB, 0xCD, 0xEF, 0x01, 0x01, 0x05]).unwrap();
    drop(f);

    {
        let mut db = Db::open_with(&dir.0, fast()).unwrap();
        assert_eq!(db.recovered_dropped_bytes(), 6);
        assert_eq!(db.get(b"good"), Some(b"yes".to_vec()));
        // New writes must land after the last good record, not after the garbage.
        db.put(b"after", b"crash").unwrap();
    }

    let db = Db::open(&dir.0).unwrap();
    assert_eq!(db.recovered_dropped_bytes(), 0);
    assert_eq!(db.get(b"after"), Some(b"crash".to_vec()));
}

#[test]
fn scan_returns_live_keys_in_order() {
    let dir = TempDir::new("scan");
    let mut db = Db::open_with(&dir.0, fast()).unwrap();
    for (k, v) in [("c", "3"), ("a", "1"), ("d", "4"), ("b", "2")] {
        db.put(k.as_bytes(), v.as_bytes()).unwrap();
    }
    db.delete(b"c").unwrap();

    let all: Vec<_> = db
        .scan(None, None)
        .into_iter()
        .map(|(k, _)| String::from_utf8(k).unwrap())
        .collect();
    assert_eq!(all, ["a", "b", "d"]);

    let some = db.scan(Some(b"b"), Some(b"d"));
    assert_eq!(some, vec![(b"b".to_vec(), b"2".to_vec())]);

    assert!(db.scan(Some(b"z"), Some(b"a")).is_empty());
}

#[test]
fn binary_keys_and_empty_values() {
    let dir = TempDir::new("binary");
    {
        let mut db = Db::open_with(&dir.0, fast()).unwrap();
        db.put(&[0, 255, 10, 13], b"").unwrap();
    }
    let db = Db::open(&dir.0).unwrap();
    assert_eq!(db.get(&[0, 255, 10, 13]), Some(vec![]));
}
