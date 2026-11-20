//! Error type for the whole crate.
//!
//! Rust has no exceptions. A function that can fail returns `Result<T, E>`,
//! which is either `Ok(value)` or `Err(error)`. The `?` operator returns the
//! error to the caller early, which keeps code short without hiding failures.

use std::fmt;
use std::io;

/// Everything that can go wrong inside tiny-lsm.
#[derive(Debug)]
pub enum Error {
    /// The operating system reported a problem (disk full, permission denied...).
    Io(io::Error),
    /// A file on disk does not have the shape we expect.
    Corrupt(String),
}

/// Shorthand so every function can say `Result<T>` instead of `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Corrupt(msg) => write!(f, "corrupt data: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Corrupt(_) => None,
        }
    }
}

/// This `From` impl is what lets `?` turn an `io::Error` into our `Error`
/// automatically.
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
