//! Landing a tree's index file in the heap: the one write path every index takes.

use std::fs;
use std::io;

use crate::repo;

/// Writes `bytes` as the index file for `hash`: staged in a private temp dir, then renamed into its shard dir,
/// so the index path never holds a partial file.
///
/// The tree must already be in the treecas: no index should ever name a tree the heap lacks.
///
/// The temp dir is made fresh for each call, inside the repo dir:
/// concurrent writers (threads included) can't collide on a temp name,
/// the rename never crosses a filesystem,
/// and the temp dir is removed on return, whether the landing succeeded or not.
/// Shard dirs are created only after a `NotFound` says they're missing, same as blob placement.
/// Renaming over an existing index is fine: the format is deterministic, so it holds these same bytes.
pub(crate) fn land_index(repo: &repo::Repo, hash: &gittree::Hash, bytes: &[u8]) -> io::Result<()> {
	let staging = tempdir::TempDir::new_in(repo.repo_path(), ".staging-index-")?;
	let staged = staging.path().join("index");
	fs::write(&staged, bytes)?;
	let dest = repo.index_path(hash);
	match fs::rename(&staged, &dest) {
		Err(e) if e.kind() == io::ErrorKind::NotFound => {
			repo.create_index_shard(hash)?;
			fs::rename(&staged, &dest)
		},
		r => r,
	}
}
