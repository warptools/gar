//! Fetching a tree from a remote heap into the local one, trusting the remote for nothing.
//!
//! The remote's index is checked by arithmetic before any blob moves:
//! its directory hashes must recompute from their entries, and its root must be the tree asked for.
//! Only blobs the local heap lacks are fetched, and each is hashed as it lands.
//! The tree is built and committed only once every blob it needs is verified and in stock,
//! and the local index is the verified entries re-serialized, never the remote's bytes.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Read as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use super::fetch_error::{FetchError, Rejection};
use super::transport::{OpenError, Transport};
use crate::blob_landing;
use crate::index_landing;
use crate::layout;
use crate::repo;
use crate::tree_landing;
use crate::treeidx;

/// The largest index a fetch will read.
///
/// The index is the one body that's read whole into memory (the parser needs all of it), so its size needs a bound.
/// An entry costs about 100 bytes plus its path, so this admits a tree of a couple of million entries:
/// several times the largest source trees around (chromium's checkout is about 400 thousand files).
/// A bigger index fails the fetch as a transport failure would: it isn't a lie, just more than we'll hold.
const MAX_INDEX_BYTES: u64 = 256 << 20;

/// What a successful fetch did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
	/// The tree was already in the local heap, so nothing was read from the remote.
	AlreadyInStock,
	/// The tree was fetched.  Blobs are counted as blobcas entries:
	/// one per distinct content and exec bit, however many paths share it.
	Fetched {
		blobs_fetched: usize,
		bytes_fetched: u64,
		blobs_in_stock: usize,
	},
}

/// Fetches the tree `hash` from `remote` into `repo`, fetching only the blobs `repo` lacks.
///
/// The repo's directories must exist (see `Repo::create`).
/// On success the tree and its index are in the heap.
/// On failure, see `FetchError` for what's left behind.
pub fn fetch_tree(
	repo: &repo::Repo,
	remote: &dyn Transport,
	hash: &gittree::Hash,
) -> Result<Outcome, FetchError> {
	// Presence means the entry directory exists, the same check `gar path` makes.
	if repo.tree_path(hash).is_dir() {
		return Ok(Outcome::AlreadyInStock);
	}

	let idx = fetch_index(remote, hash)?;
	let outcome = stock_blobs(repo, remote, &idx)?;
	let wiptree = tree_landing::new_wiptree(repo)?;
	materialize(repo, &idx, wiptree.path())?;
	tree_landing::commit_tree(repo, &wiptree, hash)?;
	index_landing::land_index(repo, hash, &treeidx::serialize(&idx))?;
	Ok(outcome)
}

/// Gets every blob the index names into the local blobcas, fetching only those not already there,
/// and checks every size claim the index makes against the blob's true size.
fn stock_blobs(
	repo: &repo::Repo,
	remote: &dyn Transport,
	idx: &treeidx::TreeIdx,
) -> Result<Outcome, FetchError> {
	// Each fetched blob is staged here before publishing; it's removed on return, success or not.
	let staging = tempdir::TempDir::new_in(repo.repo_path(), ".staging-fetch-")?;
	let mut blobs_fetched = 0;
	let mut bytes_fetched = 0;
	let mut blobs_in_stock = 0;
	// The true size of each blob this fetch has seen, by (hash, executable).
	let mut sizes: HashMap<(gittree::Hash, bool), u64> = HashMap::new();
	for entry in idx.entries() {
		let treeidx::Kind::File {
			executable,
			size: claimed,
			hash: blob,
		} = &entry.kind
		else {
			continue;
		};
		let key = (blob.clone(), *executable);
		let actual = match sizes.get(&key) {
			Some(&actual) => actual,
			None => {
				let actual = match local_blob_size(repo, blob, *executable)? {
					Some(actual) => {
						blobs_in_stock += 1;
						actual
					},
					None => {
						let staged = staging.path().join("staged");
						let actual =
							fetch_blob(repo, remote, blob, *executable, *claimed, &staged)?;
						blobs_fetched += 1;
						bytes_fetched += actual;
						actual
					},
				};
				sizes.insert(key, actual);
				actual
			},
		};
		// The index we write must be exact, and tree hashes don't cover sizes, so every claim is checked here.
		// A blob's hash commits to its length, so whatever is in stock under that name has the true size.
		if actual != *claimed {
			return Err(FetchError::Refused {
				location: remote.location(&layout::index(idx.root_hash())),
				rejection: Rejection::SizeMismatch {
					path: entry.path.clone(),
					claimed: *claimed,
					actual,
				},
			});
		}
	}

	Ok(Outcome::Fetched {
		blobs_fetched,
		bytes_fetched,
		blobs_in_stock,
	})
}

/// Reads, parses, and checks the remote's index for `hash`.  No blob is touched until this has passed.
fn fetch_index(
	remote: &dyn Transport,
	hash: &gittree::Hash,
) -> Result<treeidx::TreeIdx, FetchError> {
	let rel = layout::index(hash);
	let location = remote.location(&rel);
	let mut body = match remote.open(&rel) {
		Ok(body) => body,
		Err(OpenError::NotFound {
			location,
		}) => {
			return Err(FetchError::NotServed {
				location,
			});
		},
		Err(e) => return Err(open_failure(e)),
	};
	let bytes = read_bounded(&mut body, MAX_INDEX_BYTES)
		.map_err(|e| io::Error::new(e.kind(), format!("reading {location}: {e}")))?;
	let idx = match treeidx::parse(&bytes) {
		Ok(idx) => idx,
		Err(refusal) => {
			return Err(FetchError::Refused {
				location,
				rejection: Rejection::BadIndex(refusal),
			});
		},
	};
	if idx.root_hash() != hash {
		return Err(FetchError::Refused {
			location,
			rejection: Rejection::WrongTree {
				described: idx.root_hash().clone(),
			},
		});
	}
	Ok(idx)
}

/// Reads `reader` to its end, refusing to hold more than `limit` bytes.
///
/// Reading one byte past the limit is how an oversized body is told apart from one exactly at it.
fn read_bounded(reader: &mut dyn io::Read, limit: u64) -> io::Result<Vec<u8>> {
	let mut bytes = Vec::new();
	reader.take(limit + 1).read_to_end(&mut bytes)?;
	if bytes.len() as u64 > limit {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!("the body is larger than the {limit}-byte limit for an index"),
		));
	}
	Ok(bytes)
}

/// The size of the local blobcas entry for this blob, or `None` if there isn't one.
///
/// Anything at the path counts as present, the way backfill counts an index as present:
/// judging the local heap's own entries is repair's job, not fetch's.
fn local_blob_size(
	repo: &repo::Repo,
	hash: &gittree::Hash,
	executable: bool,
) -> io::Result<Option<u64>> {
	match fs::symlink_metadata(repo.blob_path(hash, executable)) {
		Ok(meta) => Ok(Some(meta.len())),
		Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(e),
	}
}

/// Fetches one blob into the blobcas, through a staging file at `staged`, returning its size.
///
/// Nothing is published unless it hashes to `hash`.
/// The body is read at most one byte past `claimed`: that bounds a body that never ends,
/// and an honest body reaches its EOF inside the bound, which is what returns an HTTP connection to its pool.
/// A body shorter than `claimed` that does hash correctly is the real blob, and is published;
/// the size claim itself is the caller's to judge.
fn fetch_blob(
	repo: &repo::Repo,
	remote: &dyn Transport,
	hash: &gittree::Hash,
	executable: bool,
	claimed: u64,
	staged: &Path,
) -> Result<u64, FetchError> {
	let rel = layout::blob(hash, executable);
	let location = remote.location(&rel);
	let body = match remote.open(&rel) {
		Ok(body) => body,
		Err(OpenError::NotFound {
			location,
		}) => {
			return Err(FetchError::BlobMissing {
				location,
			});
		},
		Err(e) => return Err(open_failure(e)),
	};
	let mut bounded = ReadNaming {
		inner: body.take(claimed + 1),
		location: &location,
	};
	let (computed, size) = blob_landing::stage_blob(&mut bounded, staged, executable)?;
	if size > claimed {
		return Err(FetchError::Refused {
			location,
			rejection: Rejection::BodyTooLong {
				claimed,
			},
		});
	}
	if computed != *hash {
		return Err(FetchError::Refused {
			location,
			rejection: Rejection::WrongContent {
				computed,
			},
		});
	}
	blob_landing::place_blob(repo, staged, hash, executable)?;
	fs::remove_file(staged)?;
	Ok(size)
}

/// Builds the tree the index describes in `wiptree`: directories, hardlinks to the blobcas, and symlinks.
///
/// Every blob the index names must already be in the blobcas.
/// Verified entries come in preorder with no `..` or empty segments, so each parent exists before its children,
/// and nothing lands outside the wiptree.
fn materialize(repo: &repo::Repo, idx: &treeidx::TreeIdx, wiptree: &Path) -> io::Result<()> {
	for entry in idx.entries() {
		// The root entry is the wiptree itself.
		if entry.path.is_empty() {
			continue;
		}
		let dest = wiptree.join(OsStr::from_bytes(&entry.path));
		match &entry.kind {
			treeidx::Kind::Dir {
				..
			} => fs::create_dir(&dest)?,
			treeidx::Kind::File {
				executable,
				hash,
				..
			} => fs::hard_link(repo.blob_path(hash, *executable), &dest)?,
			treeidx::Kind::Symlink {
				target,
			} => std::os::unix::fs::symlink(OsStr::from_bytes(target), &dest)?,
		}
	}
	Ok(())
}

/// A transport failure, as an io error.  Its message names where it happened,
/// and the `OpenError` (with its own cause) stays reachable as the source.
fn open_failure(e: OpenError) -> FetchError {
	FetchError::Io(io::Error::other(e))
}

/// A reader whose errors name what was being read.
///
/// Read errors from a transport's stream are bare `io::Error`s (a reset connection says nothing of which URL);
/// this attaches the location, so a failed fetch says where.
struct ReadNaming<'a, R> {
	inner: R,
	location: &'a str,
}

impl<R: io::Read> io::Read for ReadNaming<'_, R> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		self.inner
			.read(buf)
			.map_err(|e| io::Error::new(e.kind(), format!("reading {}: {e}", self.location)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn read_bounded_takes_up_to_the_limit() {
		let got = read_bounded(&mut &b"12345"[..], 5).expect("exactly at the limit");
		assert_eq!(got, b"12345");
	}

	#[test]
	fn read_bounded_refuses_past_the_limit() {
		let err = read_bounded(&mut &b"123456"[..], 5).expect_err("one byte over");
		assert_eq!(err.kind(), io::ErrorKind::InvalidData);
	}
}
