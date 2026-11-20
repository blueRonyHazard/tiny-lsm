//! # tiny-lsm
//!
//! A tiny log-structured merge-tree (LSM) storage engine.
//!
//! Every write goes to two places:
//!
//! 1. the **write-ahead log** ([`wal`]): an append-only file on disk, so the
//!    write survives a crash;
//! 2. the **memtable** ([`memtable`]): a sorted map in memory, so reads are fast.
//!
//! When the process restarts, [`Db::open`] replays the log to rebuild the
//! memtable. Later milestones add sorted files on disk (SSTables), compaction
//! and Bloom filters; see the roadmap in the README.
//!
//! ```
//! use tiny_lsm::Db;
//!
//! # let dir = std::env::temp_dir().join(format!("tiny-lsm-doc-{}", std::process::id()));
//! let mut db = Db::open(&dir)?;
//! db.put(b"language", b"rust")?;
//! assert_eq!(db.get(b"language"), Some(b"rust".to_vec()));
//!
//! db.delete(b"language")?;
//! assert_eq!(db.get(b"language"), None);
//! # std::fs::remove_dir_all(&dir).ok();
//! # Ok::<(), tiny_lsm::Error>(())
//! ```

pub mod crc32;
pub mod db;
pub mod error;
pub mod memtable;
pub mod wal;

// Re-export the types most users need, so they can write `tiny_lsm::Db`
// instead of `tiny_lsm::db::Db`.
pub use db::{Db, Options};
pub use error::{Error, Result};
