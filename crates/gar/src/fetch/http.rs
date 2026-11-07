use std::io;

use super::transport::{OpenError, Transport};
use crate::layout;

/// Reads a heap from a plain HTTP(S) server, with GETs.
///
/// Any static webserver pointed at a heap's directory will do.
/// Redirects are followed (up to ureq's default of 10).
/// Only a 404 is `NotFound`; every other unsuccessful status is a failure naming the status.
///
/// The transport owns one `ureq::Agent`, which pools keep-alive connections across opens.
/// For parallel fetching, share one transport across threads: it's `Sync`, and they all draw on that one pool.
#[derive(Debug)]
pub struct HttpTransport {
	/// The base URL, without a trailing slash: relpaths are appended after a `/`.
	base: String,
	agent: ureq::Agent,
}

impl HttpTransport {
	/// `base` is the URL of the heap's own directory (what the webserver serves as the `.gar` dir).
	/// It's used as a plain prefix: relpaths are appended to it, so it shouldn't carry a query or fragment.
	pub fn new(base: &str) -> Self {
		HttpTransport {
			base: base.strip_suffix('/').unwrap_or(base).to_owned(),
			agent: ureq::Agent::new_with_defaults(),
		}
	}
}

impl Transport for HttpTransport {
	fn open(&self, path: &layout::RelPath) -> Result<Box<dyn io::Read + Send>, OpenError> {
		// Relpaths are bare layout spellings (hex, `/`, `-x`), so they need no percent-encoding.
		let url = self.location(path);
		match self.agent.get(&url).call() {
			Ok(response) => Ok(Box::new(response.into_body().into_reader())),
			Err(ureq::Error::StatusCode(404)) => Err(OpenError::NotFound {
				location: url,
			}),
			Err(e) => Err(OpenError::Failed {
				location: url,
				cause: e.into_io(),
			}),
		}
	}

	fn location(&self, path: &layout::RelPath) -> String {
		format!("{}/{}", self.base, path)
	}
}
