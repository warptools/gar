use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Repo {
	/// Path to the repo's own directory.
	/// Includes ".gar": `new` takes the directory that contains the repo, and appends that.
	/// Usually this is what's used in messages to the user.
	path: PathBuf,

	// All of the paths below are suffixes of 'self.path', but used so frequently we create them once.
	blobcas_path: PathBuf,
	treecas_path: PathBuf,
	treeidx_path: PathBuf,
}

impl Repo {
	pub fn new(root_path: impl AsRef<Path>) -> Self {
		let path = root_path.as_ref().join(".gar");
		Repo {
			blobcas_path: (&path).join("blobcas"),
			treecas_path: (&path).join("treecas"),
			treeidx_path: (&path).join("treeidx"),
			path,
		}
	}

	/// Creates the repo's directories, if they're not already there.
	///
	/// The directory containing the repo must already exist; see `init_at` for making that.
	///
	/// Idempotent, so this is also how a repo missing a subdirectory gets restored.
	pub fn create(&self) -> io::Result<()> {
		for dir in [
			self.repo_path(),
			self.blobcas_path(),
			self.treecas_path(),
			self.treeidx_path(),
		] {
			create_dir_if_absent(dir)?;
		}
		Ok(())
	}

	pub fn repo_path(&self) -> &Path {
		&self.path
	}

	/// Reports whether this repo is actually present on disk.
	///
	/// This asks whether the `.gar` path is a directory, not merely whether something is there:
	/// a plain file named `.gar` is not a repo.
	pub fn exists(&self) -> bool {
		self.repo_path().is_dir()
	}
	pub fn blobcas_path(&self) -> &Path {
		&self.blobcas_path
	}
	pub fn treecas_path(&self) -> &Path {
		&self.treecas_path
	}
	pub fn treeidx_path(&self) -> &Path {
		&self.treeidx_path
	}
}

/// `fs::create_dir`, counting an already-present directory as success.
fn create_dir_if_absent(p: &Path) -> io::Result<()> {
	match fs::create_dir(p) {
		Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
		r => r,
	}
}

/// Creates a repo in `dir`, making `dir` itself if it's not there yet -- but nothing above it.
///
/// One missing directory is a reasonable thing to ask for; a missing chain of them is
/// nearly always a mistyped path, and answering that by manifesting four new directories
/// is not a favor to anyone.
pub fn init_at(dir: impl AsRef<Path>) -> Result<Repo, io::Error> {
	let dir = dir.as_ref();
	match create_dir_if_absent(dir) {
		Ok(()) => {},
		Err(e) if e.kind() == io::ErrorKind::NotFound => {
			// The bare io error names no path at all, which makes for a mystifying report
			// of what is usually a typo.
			return Err(io::Error::new(
				io::ErrorKind::NotFound,
				format!(
					"cannot create gar repo in {:?}: its containing directory does not exist",
					dir,
				),
			));
		},
		Err(e) => return Err(e),
	}
	let repo = Repo::new(dir);
	repo.create()?;
	Ok(repo)
}

/// Searches for the directory holding a gar repo: `dir` itself, or the nearest of its ancestors.
///
/// `dir` must be absolute and already resolved.
/// Walking upward from a relative path runs out at "" long before reaching the filesystem root,
/// which is a wrong answer rather than a slow one -- so relative input is refused instead.
pub fn find_repo_dir(dir: &Path) -> Result<Option<PathBuf>, io::Error> {
	if !dir.is_absolute() {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			format!("cannot search for a gar repo from the relative path {dir:?}"),
		));
	}
	for candidate in dir.ancestors() {
		if candidate.join(".gar").exists() {
			return Ok(Some(candidate.to_owned()));
		}
	}
	Ok(None)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
	fn scratch(name: &str) -> tempdir::TempDir {
		tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
	}

	#[test]
	fn create_makes_the_repo_dirs() {
		let td = scratch("create");
		let repo = Repo::new(td.path());
		repo.create().expect("creating a repo");
		assert!(repo.blobcas_path().is_dir());
		assert!(repo.treecas_path().is_dir());
		assert!(repo.treeidx_path().is_dir());
	}

	#[test]
	fn init_at_makes_the_named_dir() {
		let td = scratch("init-at");
		let repo = init_at(td.path().join("fresh")).expect("creating a repo one level down");
		assert!(repo.blobcas_path().is_dir());
	}

	#[test]
	fn init_at_refuses_a_missing_chain() {
		let td = scratch("init-at-chain");
		let err = init_at(td.path().join("no/such/deep/place"))
			.expect_err("creating a repo four levels down");
		assert_eq!(err.kind(), io::ErrorKind::NotFound);
		assert!(
			err.to_string().contains("no/such/deep"),
			"the message should name the directory asked for, got: {err}"
		);
		assert!(
			!td.path().join("no").exists(),
			"not one directory of the missing chain should have been created"
		);
	}

	#[test]
	fn create_is_idempotent_and_restores_subdirs() {
		let td = scratch("create-idempotent");
		let repo = Repo::new(td.path());
		repo.create().expect("creating a repo");
		fs::remove_dir(repo.treeidx_path()).expect("removing a subdir");
		repo.create().expect("creating a repo that mostly exists");
		assert!(repo.treeidx_path().is_dir());
	}

	#[test]
	fn find_repo_dir_finds_the_dir_itself() {
		let td = scratch("find-self");
		init_at(td.path()).expect("creating a repo");
		let found = find_repo_dir(td.path()).expect("no io errors");
		assert_eq!(found.as_deref(), Some(td.path()));
	}

	#[test]
	fn find_repo_dir_finds_an_ancestor() {
		let td = scratch("find-ancestor");
		init_at(td.path()).expect("creating a repo");
		let deep = td.path().join("a/b/c");
		fs::create_dir_all(&deep).expect("can make subdirs");
		let found = find_repo_dir(&deep).expect("no io errors");
		assert_eq!(found.as_deref(), Some(td.path()));
	}

	#[test]
	fn find_repo_dir_finds_nothing_when_there_is_nothing() {
		let td = scratch("find-nothing");
		let deep = td.path().join("a/b/c");
		fs::create_dir_all(&deep).expect("can make subdirs");
		let found = find_repo_dir(&deep).expect("no io errors");
		// Asserted as "nothing in our scratch tree" rather than "nothing at all":
		// the walk continues past the tempdir to the filesystem root, and what lives up
		// there is the machine's business, not this test's.
		assert!(
			found.as_ref().map_or(true, |d| !d.starts_with(td.path())),
			"found a repo inside the scratch tree: {found:?}"
		);
	}

	/// An upward walk of a relative path would end at "" rather than at the filesystem root,
	/// reporting "no repo" for a directory that's sitting inside one.
	#[test]
	fn find_repo_dir_refuses_relative_input() {
		let err = find_repo_dir(Path::new("some/relative/path")).expect_err("relative input");
		assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
	}
}
