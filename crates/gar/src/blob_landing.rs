//! Landing blobs in the blobcas: the publishing step every blob writer shares.

use std::fs;
use std::io;
use std::path::Path;

use crate::repo;

/// Makes the blobcas entry for `hash` a hardlink to `source` -- or accepts whatever is already there.
///
/// A CAS name is never replaced once it exists: replacing it would split the inode family
/// shared with existing treecas links, and a racing writer can see the swap as a `NotFound`
/// in the middle of its own linking.  `link` never clobbers, so an `AlreadyExists` from it
/// means the blob is already stored, and that's dedup doing its job: success.
///
/// `source` must already carry the blob's final mode: a blobcas inode is never touched again once published.
///
/// Placement is optimistic: shard directories are created only after a `NotFound` says they're missing.
/// That trades one failed `link` on each shard's first blob for skipping two `mkdir` calls on every other.
pub(crate) fn place_blob(
	repo: &repo::Repo,
	source: &Path,
	hash: &gittree::Hash,
	executable: bool,
) -> io::Result<()> {
	let blobcas_path = repo.blob_path(hash, executable);
	match fs::hard_link(source, &blobcas_path) {
		Err(e) if e.kind() == io::ErrorKind::NotFound => {
			repo.create_blob_shard(hash)?;
			match fs::hard_link(source, &blobcas_path) {
				Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
				r => r,
			}
		},
		Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
		r => r,
	}
}
