use std::fmt;
use std::io;

use crate::treeidx;

/// Why a fetch stopped.
///
/// The lanes are what a caller acts on differently:
/// the remote not having the tree (`NotServed`, `BlobMissing`),
/// the remote serving something that doesn't verify (`Refused`),
/// and everything else, transport and local filesystem alike (`Io`).
///
/// Whatever the lane, nothing unverified has landed, and no tree or index has been committed.
/// Blobs that verified before the failure stay in the blobcas, so a rerun picks up where this one stopped.
#[derive(Debug)]
pub enum FetchError {
	/// The remote has no index for the tree, so it doesn't serve that tree.
	NotServed {
		location: String,
	},
	/// The remote's index verified, but a blob it names isn't there.
	BlobMissing {
		location: String,
	},
	/// What the remote served at `location` failed verification.
	Refused {
		location: String,
		rejection: Rejection,
	},
	Io(io::Error),
}

/// What was wrong with something the remote served.
#[derive(Debug)]
pub enum Rejection {
	/// The index isn't a valid treeidx, or its directory hashes don't recompute from their entries.
	BadIndex(treeidx::Refusal),
	/// The index is valid, but it describes a different tree than the one asked for.
	WrongTree {
		described: gittree::Hash,
	},
	/// The index claims a size for a file that its blob doesn't have.
	/// (Tree hashes don't cover sizes, so this is only caught when the blob's real size is known.)
	SizeMismatch {
		path: Vec<u8>,
		claimed: u64,
		actual: u64,
	},
	/// A blob's body ran past the size the index claims for it, and reading stopped there.
	BodyTooLong {
		claimed: u64,
	},
	/// A blob's body doesn't hash to the blob's name.
	WrongContent {
		computed: gittree::Hash,
	},
}

impl From<io::Error> for FetchError {
	fn from(e: io::Error) -> Self {
		FetchError::Io(e)
	}
}

impl fmt::Display for FetchError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			FetchError::NotServed {
				location,
			} => write!(f, "that tree isn't served there: no index at {location}"),
			FetchError::BlobMissing {
				location,
			} => write!(
				f,
				"the remote's index advertises blobs it doesn't hold: nothing at {location}"
			),
			FetchError::Refused {
				location,
				rejection,
			} => write!(f, "refusing {location}: {rejection}"),
			FetchError::Io(e) => e.fmt(f),
		}
	}
}

impl fmt::Display for Rejection {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Rejection::BadIndex(refusal) => refusal.fmt(f),
			Rejection::WrongTree {
				described,
			} => write!(
				f,
				"the index describes tree {}, not the tree asked for",
				described.as_hex()
			),
			Rejection::SizeMismatch {
				path,
				claimed,
				actual,
			} => write!(
				f,
				"the index claims \"./{}\" is {claimed} bytes, but its blob is {actual} bytes",
				path.escape_ascii()
			),
			Rejection::BodyTooLong {
				claimed,
			} => write!(
				f,
				"the body runs past the {claimed} bytes the index claims for it"
			),
			Rejection::WrongContent {
				computed,
			} => write!(
				f,
				"the body hashes to {}, not to the blob's name",
				computed.as_hex()
			),
		}
	}
}
