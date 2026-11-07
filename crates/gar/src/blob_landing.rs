//! Landing blobs in the blobcas: staging a private copy, and publishing it under its hash.

use std::fs;
use std::io;
use std::io::Seek as _;
use std::os::unix::fs::PermissionsExt as _;
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

/// Copies `source` into a new file at `staged_path`, gives it the blobcas mode, and hashes the copy.
/// Returns the hash and size of what landed, ready for `place_blob`.
///
/// Copy first, hash the copy afterward -- don't tee a hash out of the copy, and don't hash the source:
/// the plain copy takes kernel fast paths that make it plus a re-read cheaper than streaming
/// through userland (see `experiments/benches/direct_copy.rs` for evidence),
/// and hashing the bytes we actually publish means a source that changes mid-read
/// still can't put content under a CAS name that doesn't describe it.
/// (Generic rather than `dyn`, so `io::copy` still sees a concrete `File` and can take those fast paths.)
///
/// `staged_path` must not exist yet; the caller owns its cleanup.
pub(crate) fn stage_blob<R: io::Read + ?Sized>(
	source: &mut R,
	staged_path: &Path,
	executable: bool,
) -> io::Result<(gittree::Hash, u64)> {
	let mut staged = fs::OpenOptions::new()
		.read(true)
		.write(true)
		.create_new(true)
		.open(staged_path)?;
	let size = io::copy(source, &mut staged)?;
	// This is an fchmod, so the published mode is exact, umask notwithstanding.
	// It must happen before publishing: a blobcas inode is never touched again.
	let mode = if executable {
		repo::BLOB_MODE_EXEC
	} else {
		repo::BLOB_MODE
	};
	staged.set_permissions(fs::Permissions::from_mode(mode))?;
	staged.seek(io::SeekFrom::Start(0))?;
	let hash = gittree::hash_of_stream(&mut staged, size)?;
	Ok((hash, size))
}
