use std::fmt;
use std::io;

use crate::layout;

/// Somewhere a heap's files can be read from, by heap-relative path.
///
/// Object-safe, so a caller can hold a `Box<dyn Transport>` picked at runtime (see `Remote`).
/// `Send + Sync` so one transport can be shared by worker threads.
pub trait Transport: Send + Sync {
	/// Opens the file at `path` in the remote heap, as a stream.
	///
	/// Bodies are never buffered whole: the reader pulls from the source as it's read.
	/// Read to EOF -- until a read returns 0 -- whenever the body is wanted in full.
	/// For HTTP, that final read is what returns the connection to the pool;
	/// a reader dropped short of it closes its connection instead.
	/// (`io::copy`, `read_to_end`, and `gittree::hash_of_stream` all read to EOF.)
	///
	/// Errors while reading the stream are plain `io::Error`s;
	/// `location` names what was being read, for a caller reporting them.
	fn open(&self, path: &layout::RelPath) -> Result<Box<dyn io::Read + Send>, OpenError>;

	/// Names where `path` is in this remote: the full URL or filesystem path, for diagnostics.
	fn location(&self, path: &layout::RelPath) -> String;
}

/// Why a transport couldn't open a file.
///
/// `NotFound` is the remote cleanly reporting it has nothing at that path.
/// Every other problem is `Failed`, and nothing about it should be read as "absent".
#[derive(Debug)]
pub enum OpenError {
	NotFound {
		location: String,
	},
	Failed {
		location: String,
		cause: io::Error,
	},
}

impl fmt::Display for OpenError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			OpenError::NotFound {
				location,
			} => write!(f, "{location}: not found"),
			OpenError::Failed {
				location,
				cause,
			} => write!(f, "{location}: {cause}"),
		}
	}
}

impl std::error::Error for OpenError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			OpenError::NotFound {
				..
			} => None,
			OpenError::Failed {
				cause,
				..
			} => Some(cause),
		}
	}
}
