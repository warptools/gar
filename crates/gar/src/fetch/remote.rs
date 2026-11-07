use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

use super::http::HttpTransport;
use super::local::LocalTransport;
use super::transport::Transport;

/// A remote heap's base, classified by its spelling: an `http://` or `https://` prefix is a URL,
/// and anything else is a local filesystem path.
///
/// The base names the heap's own directory (the `.gar` dir), which directly contains `blobcas/` and `treeidx/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remote {
	Local(PathBuf),
	Http(String),
}

impl Remote {
	/// Classifies `base`.  Scheme matching is exact (lowercase), with no other probing.
	///
	/// Refuses only a URL-prefixed base that isn't UTF-8: it can't be a valid URL,
	/// and reading it as a local path would just turn a typo into a confusing "not found".
	pub fn parse(base: &OsStr) -> io::Result<Remote> {
		let bytes = base.as_encoded_bytes();
		if !(bytes.starts_with(b"http://") || bytes.starts_with(b"https://")) {
			return Ok(Remote::Local(PathBuf::from(base)));
		}
		match base.to_str() {
			Some(url) => Ok(Remote::Http(url.to_owned())),
			None => Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				format!("{base:?} starts like a URL, but isn't valid UTF-8"),
			)),
		}
	}

	pub fn into_transport(self) -> Box<dyn Transport> {
		match self {
			Remote::Local(path) => Box::new(LocalTransport::new(path)),
			Remote::Http(url) => Box::new(HttpTransport::new(&url)),
		}
	}
}
