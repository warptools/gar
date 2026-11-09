//! Offered files: local files the caller says may be blobs the tree needs, and vouches will never change again.
//!
//! An offer that hashes to a blob the heap lacks is hardlinked into the blobcas in place of fetching that blob.
//! The rest are never opened for anything but reading.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use crate::blob_landing;
use crate::repo;

/// The offered files, by the hash each had when offered.
pub(super) struct Offers<'a> {
	by_hash: HashMap<gittree::Hash, &'a Path>,
	pub(super) used: Vec<PathBuf>,
}

impl<'a> Offers<'a> {
	/// Hashes every offered file, refusing any that `link_in` couldn't link into `repo`:
	/// anything but a regular file, or a file on another filesystem than the heap.
	pub(super) fn gather(repo: &repo::Repo, paths: &'a [PathBuf]) -> io::Result<Self> {
		let heap_dev = fs::metadata(repo.repo_path())?.dev();
		let mut by_hash = HashMap::new();
		for path in paths {
			let meta = fs::symlink_metadata(path).map_err(|e| naming(path, e))?;
			if !meta.is_file() {
				return Err(naming(
					path,
					io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"),
				));
			}
			if meta.dev() != heap_dev {
				return Err(naming(
					path,
					io::Error::new(
						io::ErrorKind::CrossesDevices,
						"on a different filesystem than the heap, so it can't be linked in",
					),
				));
			}
			let hash = gittree::hash_of_path(path).map_err(|e| naming(path, e))?;
			by_hash.insert(hash, path.as_path());
		}
		Ok(Offers {
			by_hash,
			used: Vec::new(),
		})
	}

	/// Puts the offer that hashed to `hash`, if there was one, into the blobcas as (`hash`, `executable`),
	/// through a hardlink at `staged`, and returns its size.  `None` when nothing was offered under that hash.
	///
	/// The offer is hashed again through the staged link, so what is published is what was hashed;
	/// a file that changed since it was offered fails the fetch.
	pub(super) fn link_in(
		&mut self,
		repo: &repo::Repo,
		hash: &gittree::Hash,
		executable: bool,
		staged: &Path,
	) -> io::Result<Option<u64>> {
		let Some(path) = self.by_hash.remove(hash) else {
			return Ok(None);
		};
		fs::hard_link(path, staged).map_err(|e| naming(path, e))?;
		let (computed, size) = blob_landing::seal_link(staged, executable)?;
		if computed != *hash {
			return Err(naming(
				path,
				io::Error::new(
					io::ErrorKind::InvalidData,
					"its content changed after it was offered",
				),
			));
		}
		blob_landing::place_blob(repo, staged, hash, executable)?;
		fs::remove_file(staged)?;
		self.used.push(path.to_owned());
		Ok(Some(size))
	}
}

/// An error about an offer, naming it: io errors on the file itself don't say which file.
fn naming(path: &Path, e: io::Error) -> io::Error {
	io::Error::new(e.kind(), format!("offer {}: {e}", path.display()))
}
