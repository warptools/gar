//! Landing trees in the treecas: a wiptree is built privately, then renamed into its CAS name whole.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::repo;

/// Makes a fresh, empty wiptree to build a tree in.  It's removed on drop, unless `commit_tree` has moved it.
///
/// It sits in the treecas root, so the commit's rename never crosses a filesystem,
/// and its name is dot-prefixed, so walkers of the treecas never mistake it for a tree.
pub(crate) fn new_wiptree(repo: &repo::Repo) -> io::Result<tempdir::TempDir> {
	tempdir::TempDir::new_in(repo.treecas_path(), ".wiptree-")
}

/// Moves a finished wiptree into its CAS place, under `hash`.
///
/// Shard dirs are created only after a `NotFound` says they're missing, same as blob placement.
///
/// A failed rename is tolerated if a directory holds the destination: in a CAS, losing
/// the rename race to another writer manifesting the same tree is success.  The check is on the
/// destination afterward, not the error kind, because the kinds don't carve this cleanly:
/// POSIX allows either `EEXIST` or `ENOTEMPTY` for a nonempty directory target, and a rename
/// onto an existing *empty* directory (the empty tree) succeeds outright.  "Is a directory
/// there now?" is the actual question.  A non-directory squatting at the name is not that
/// tree -- returning success would hand back a hash for a heap that lacks it -- so that,
/// and every other failure, stays an error.
pub(crate) fn commit_tree(
	repo: &repo::Repo,
	wiptree: &tempdir::TempDir,
	hash: &gittree::Hash,
) -> io::Result<()> {
	let dest_path: PathBuf = repo.tree_path(hash);
	let mut treecas_result = fs::rename(wiptree, &dest_path);
	if let Err(e) = &treecas_result {
		if e.kind() == io::ErrorKind::NotFound {
			repo.create_tree_shard(hash)?;
			treecas_result = fs::rename(wiptree, &dest_path);
		}
	}
	if let Err(e) = treecas_result {
		if !dest_path.is_dir() {
			return Err(e);
		}
	}
	Ok(())
}
