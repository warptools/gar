use std::fs;
use std::io;
use std::path::PathBuf;

use super::transport::{OpenError, Transport};
use crate::layout;

/// Reads a heap from the local filesystem.
#[derive(Debug)]
pub struct LocalTransport {
	base: PathBuf,
}

impl LocalTransport {
	/// `base` is the heap's own directory (the `.gar` dir).
	/// A relative base is resolved against the working directory at each open.
	pub fn new(base: impl Into<PathBuf>) -> Self {
		LocalTransport {
			base: base.into(),
		}
	}

	/// Only `NotFound` reads as absent.  A missing shard dir is that too, as the layout specifies.
	fn open_error(&self, path: &layout::RelPath, cause: io::Error) -> OpenError {
		let location = self.location(path);
		match cause.kind() {
			io::ErrorKind::NotFound => OpenError::NotFound {
				location,
			},
			_ => OpenError::Failed {
				location,
				cause,
			},
		}
	}
}

impl Transport for LocalTransport {
	/// Anything but a regular file at `path` is a failure, caught before opening:
	/// a directory would otherwise open fine and only fail at the first read,
	/// and a fifo would block the open indefinitely.
	/// Symlinks are followed, as a static webserver serving this directory would.
	fn open(&self, path: &layout::RelPath) -> Result<Box<dyn io::Read + Send>, OpenError> {
		let full = self.base.join(path);
		let failed = |cause| self.open_error(path, cause);
		let meta = fs::metadata(&full).map_err(failed)?;
		if !meta.is_file() {
			return Err(failed(io::Error::other("not a regular file")));
		}
		let file = fs::File::open(&full).map_err(failed)?;
		Ok(Box::new(file))
	}

	fn location(&self, path: &layout::RelPath) -> String {
		self.base.join(path).display().to_string()
	}
}
