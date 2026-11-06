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

	/// Where a blob lives in the blobcas.  Executable blobs are stored separately, with a `-x` suffix.
	pub fn blob_path(&self, hash: &gittree::Hash, executable: bool) -> PathBuf {
		let mut name = hash.as_hex();
		if executable {
			name.push_str("-x");
		}
		shard_dir(&self.blobcas_path, hash).join(name)
	}

	/// Where a tree lives in the treecas.
	pub fn tree_path(&self, hash: &gittree::Hash) -> PathBuf {
		shard_dir(&self.treecas_path, hash).join(hash.as_hex())
	}

	/// Creates the shard dirs that `blob_path` for this hash sits in, if they're not already there.
	pub fn create_blob_shard(&self, hash: &gittree::Hash) -> io::Result<()> {
		create_shard(&self.blobcas_path, hash)
	}

	/// Creates the shard dirs that `tree_path` for this hash sits in, if they're not already there.
	pub fn create_tree_shard(&self, hash: &gittree::Hash) -> io::Result<()> {
		create_shard(&self.treecas_path, hash)
	}
}

/// The directory a CAS entry sits in: `{cas_root}/{ab}/{cd}`, from the first four hex chars of the hash.
///
/// Sharding keeps any one directory from accumulating millions of entries.
/// The layout is specified in README_formats.md.
fn shard_dir(cas_root: &Path, hash: &gittree::Hash) -> PathBuf {
	let hex = hash.as_hex();
	cas_root.join(&hex[0..2]).join(&hex[2..4])
}

/// Creates the two shard levels under `cas_root` for this hash.
///
/// Shard dirs are made lazily, as entries arrive, rather than all 65536 at init.
/// Concurrent adds may race to make the same shard, and that's fine.
/// Nothing above the shard levels is made: a missing CAS root stays an error.
fn create_shard(cas_root: &Path, hash: &gittree::Hash) -> io::Result<()> {
	let inner = shard_dir(cas_root, hash);
	let outer = inner
		.parent()
		.expect("shard_dir returns a path two levels below the root");
	create_dir_if_absent(outer)?;
	create_dir_if_absent(&inner)
}

/// `fs::create_dir`, counting an already-present directory as success.
fn create_dir_if_absent(p: &Path) -> io::Result<()> {
	match fs::create_dir(p) {
		// Already there is fine -- but only if it is actually a directory.
		// A plain file squatting at the path is not, and must not read as success.
		Err(e) if e.kind() == io::ErrorKind::AlreadyExists && p.is_dir() => Ok(()),
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
		// A ".gar" that is a plain file is not a repo; is_dir skips past it and keeps walking.
		if candidate.join(".gar").is_dir() {
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

	#[test]
	fn create_refuses_a_file_where_a_subdir_belongs() {
		let td = scratch("create-file-subdir");
		let repo = Repo::new(td.path());
		fs::create_dir(repo.repo_path()).expect("make the .gar dir");
		// A plain file sits where the treeidx subdir belongs.
		fs::write(repo.treeidx_path(), b"not a dir\n").expect("write the imposter");
		let err = repo
			.create()
			.expect_err("create must refuse a file where a subdir belongs");
		assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
	}

	#[test]
	fn find_repo_dir_skips_a_gar_file_and_finds_the_real_ancestor() {
		let td = scratch("find-skip-gar-file");
		init_at(td.path()).expect("creating the real repo");
		let mid = td.path().join("mid");
		fs::create_dir(&mid).expect("mkdir mid");
		// A plain file named ".gar" -- not a repo.
		fs::write(mid.join(".gar"), b"not a dir\n").expect("write the imposter");
		let deep = mid.join("deep");
		fs::create_dir(&deep).expect("mkdir deep");
		let found = find_repo_dir(&deep).expect("no io errors");
		assert_eq!(found.as_deref(), Some(td.path()));
	}

	fn some_hash() -> gittree::Hash {
		gittree::Hash::from_hex("2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce")
			.expect("valid hex")
	}

	/// A second creation is what a concurrent add racing to the same shard looks like.
	#[test]
	fn create_shard_makes_the_entry_parent_and_tolerates_repeats() {
		let td = scratch("create-shard");
		let repo = init_at(td.path()).expect("creating a repo");
		let h = some_hash();
		repo.create_blob_shard(&h).expect("first shard creation");
		repo.create_blob_shard(&h).expect("repeated shard creation");
		assert!(repo.blob_path(&h, false).parent().is_some_and(Path::is_dir));
		repo.create_tree_shard(&h).expect("tree shard creation");
		assert!(repo.tree_path(&h).parent().is_some_and(Path::is_dir));
	}

	#[test]
	fn create_shard_does_not_make_a_missing_cas_root() {
		let td = scratch("create-shard-no-root");
		let repo = init_at(td.path()).expect("creating a repo");
		fs::remove_dir(repo.blobcas_path()).expect("removing the blobcas");
		let err = repo
			.create_blob_shard(&some_hash())
			.expect_err("shard creation without a blobcas");
		assert_eq!(err.kind(), io::ErrorKind::NotFound);
		assert!(!repo.blobcas_path().exists());
	}
}
