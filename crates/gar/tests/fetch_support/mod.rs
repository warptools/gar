//! Helpers shared by the fetch test files: a specimen tree, heaps to fetch between,
//! and a transport wrapper that records what was opened and how much of each was read.

// Each test file compiles this module as its own crate, and not every file uses every helper.
#![allow(dead_code)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gar::add;
use gar::fetch;
use gar::layout;
use gar::repo;
use gar::treeidx;

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
pub fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// The size of the specimen's one large file: big enough to span many reads.
pub const BIG_SIZE: usize = 100 << 10;

/// Plants a tree with every entry kind fetch has to rebuild, at `root/{name}`:
/// - `a_file` and `copy_of_a`: one blob under two paths (fetched once);
/// - `exec_twin`: the same content again, executable (a second blobcas entry);
/// - `nested/run.sh`: executable; `nested/big`: `BIG_SIZE` bytes;
/// - `has space`, and `nested/new\nline`: awkward names;
/// - `link`: a symlink into the tree; `spaced_target`: a dangling symlink whose target has spaces;
/// - `empty/`: an empty dir; `nested/deeper/hollow/`: an empty dir inside an otherwise empty dir.
///
/// That's 7 files over 6 distinct blobs.
pub fn plant_specimen(root: &Path, name: &str) -> PathBuf {
	let dir = root.join(name);
	fs::create_dir_all(dir.join("nested/deeper/hollow")).expect("mkdir");
	fs::create_dir(dir.join("empty")).expect("mkdir");
	fs::write(dir.join("a_file"), b"a file\n").expect("write");
	fs::write(dir.join("copy_of_a"), b"a file\n").expect("write");
	fs::write(dir.join("exec_twin"), b"a file\n").expect("write");
	fs::set_permissions(dir.join("exec_twin"), fs::Permissions::from_mode(0o755)).expect("chmod");
	fs::write(dir.join("has space"), b"spaced\n").expect("write");
	fs::write(dir.join("nested/new\nline"), b"newlined\n").expect("write");
	fs::write(dir.join("nested/run.sh"), b"#!/bin/sh\necho hi\n").expect("write");
	fs::set_permissions(dir.join("nested/run.sh"), fs::Permissions::from_mode(0o755))
		.expect("chmod");
	fs::write(dir.join("nested/big"), pseudo_random(BIG_SIZE)).expect("write");
	std::os::unix::fs::symlink("nested/run.sh", dir.join("link")).expect("symlink");
	std::os::unix::fs::symlink("a target with spaces", dir.join("spaced_target")).expect("symlink");
	dir
}

/// xorshift64 bytes: no repeating pattern for a misplaced chunk to hide in.
pub fn pseudo_random(len: usize) -> Vec<u8> {
	let mut x: u64 = 0x9e3779b97f4a7c15;
	(0..len)
		.map(|_| {
			x ^= x << 13;
			x ^= x >> 7;
			x ^= x << 17;
			x as u8
		})
		.collect()
}

/// A fresh heap in `root/{name}`.
pub fn heap(root: &Path, name: &str) -> repo::Repo {
	repo::init_at(root.join(name)).expect("can make a heap")
}

/// A heap at `root/remote` holding the specimen; returns it and the specimen's treehash.
pub fn remote_with_specimen(root: &Path) -> (repo::Repo, gittree::Hash) {
	let remote = heap(root, "remote");
	let src = plant_specimen(root, "specimen");
	let hash = add::add(&remote, &src, add::FaithMode::Copy).expect("can add the specimen");
	(remote, hash)
}

/// Every file and directory directly in the heap dir and the treecas root whose name starts with a dot:
/// the staging and wiptree temps.  Fetch should leave none behind, whether it succeeded or not.
pub fn leftover_temps(heap: &repo::Repo) -> Vec<PathBuf> {
	let mut found = Vec::new();
	for dir in [heap.repo_path(), heap.treecas_path()] {
		for ent in fs::read_dir(dir).expect("can list the heap") {
			let ent = ent.expect("can read a dir entry");
			if ent.file_name().as_encoded_bytes().starts_with(b".") {
				found.push(ent.path());
			}
		}
	}
	found
}

/// Counts every blobcas entry in a heap.
pub fn blob_count(heap: &repo::Repo) -> usize {
	let mut n = 0;
	for outer in fs::read_dir(heap.blobcas_path()).expect("list blobcas") {
		for inner in fs::read_dir(outer.expect("entry").path()).expect("list shard") {
			n += fs::read_dir(inner.expect("entry").path())
				.expect("list shard")
				.count();
		}
	}
	n
}

/// One `open` a `Recording` transport saw.
pub struct Opened {
	pub rel: String,
	/// Bytes read from the body so far; zero for an open that failed.
	pub bytes_read: Arc<AtomicU64>,
}

/// Wraps a transport, recording every open, in order, and how many bytes were read from each.
pub struct Recording {
	inner: Box<dyn fetch::Transport>,
	log: Mutex<Vec<Opened>>,
}

impl Recording {
	pub fn new(inner: Box<dyn fetch::Transport>) -> Self {
		Recording {
			inner,
			log: Mutex::new(Vec::new()),
		}
	}

	/// Every relpath opened, in order.
	pub fn opens(&self) -> Vec<String> {
		let log = self.log.lock().expect("log lock");
		log.iter().map(|o| o.rel.clone()).collect()
	}

	/// The bytes read from the one open of `rel`; panics unless it was opened exactly once.
	pub fn bytes_read(&self, rel: &layout::RelPath) -> u64 {
		let log = self.log.lock().expect("log lock");
		let mut hits = log.iter().filter(|o| o.rel == rel.as_str());
		let hit = hits
			.next()
			.unwrap_or_else(|| panic!("{rel} was never opened"));
		assert!(hits.next().is_none(), "{rel} was opened more than once");
		hit.bytes_read.load(Ordering::SeqCst)
	}
}

impl fetch::Transport for Recording {
	fn open(&self, path: &layout::RelPath) -> Result<Box<dyn io::Read + Send>, fetch::OpenError> {
		let counter = Arc::new(AtomicU64::new(0));
		self.log.lock().expect("log lock").push(Opened {
			rel: path.as_str().to_owned(),
			bytes_read: counter.clone(),
		});
		let inner = self.inner.open(path)?;
		Ok(Box::new(Counting {
			inner,
			counter,
		}))
	}

	fn location(&self, path: &layout::RelPath) -> String {
		self.inner.location(path)
	}
}

struct Counting {
	inner: Box<dyn io::Read + Send>,
	counter: Arc<AtomicU64>,
}

impl io::Read for Counting {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let n = self.inner.read(buf)?;
		self.counter.fetch_add(n as u64, Ordering::SeqCst);
		Ok(n)
	}
}

/// A recording transport over the local filesystem, reading `remote`.
pub fn recording_from(remote: &repo::Repo) -> Recording {
	Recording::new(Box::new(fetch::LocalTransport::new(remote.repo_path())))
}

/// Wraps a transport, making the body of one relpath fail partway: a few bytes, then a connection reset.
pub struct BreaksMidStream {
	pub inner: Box<dyn fetch::Transport>,
	pub broken: layout::RelPath,
}

impl fetch::Transport for BreaksMidStream {
	fn open(&self, path: &layout::RelPath) -> Result<Box<dyn io::Read + Send>, fetch::OpenError> {
		let body = self.inner.open(path)?;
		if *path != self.broken {
			return Ok(body);
		}
		Ok(Box::new(io::Read::chain(
			io::Read::take(body, 3),
			FailingRead,
		)))
	}

	fn location(&self, path: &layout::RelPath) -> String {
		self.inner.location(path)
	}
}

struct FailingRead;

impl io::Read for FailingRead {
	fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
		Err(io::Error::new(
			io::ErrorKind::ConnectionReset,
			"the test broke it",
		))
	}
}

/// The remote's index for `hash`, parsed.
pub fn remote_index(remote: &repo::Repo, hash: &gittree::Hash) -> treeidx::TreeIdx {
	treeidx::parse(&fs::read(remote.index_path(hash)).expect("remote has the index"))
		.expect("remote's index parses")
}

/// The blob behind the file entry at `path`: its hash and exec bit.
pub fn blob_of(idx: &treeidx::TreeIdx, path: &str) -> (gittree::Hash, bool) {
	for entry in idx.entries() {
		if entry.path == path.as_bytes() {
			if let treeidx::Kind::File {
				hash,
				executable,
				..
			} = &entry.kind
			{
				return (hash.clone(), *executable);
			}
		}
	}
	panic!("no file entry {path:?}");
}

/// Where the local-filesystem transport says a remote entry is.
pub fn location(remote: &repo::Repo, rel: &layout::RelPath) -> String {
	remote.repo_path().join(rel).display().to_string()
}

/// Replaces the remote's index with one whose entries `edit` has changed, and that still verifies.
pub fn rewrite_remote_index(
	remote: &repo::Repo,
	hash: &gittree::Hash,
	edit: impl FnOnce(&mut Vec<treeidx::Entry>),
) {
	let mut entries = remote_index(remote, hash).into_entries();
	edit(&mut entries);
	let idx = treeidx::verify(entries).expect("the edit keeps the index verifiable");
	// Remove-then-write: the stored index is read-only, and unlike blobs it has no inode to preserve.
	let path = remote.index_path(hash);
	fs::remove_file(&path).expect("can remove the stored index");
	fs::write(&path, treeidx::serialize(&idx)).expect("can write the sabotaged index");
}

/// Sets the claimed size of the file entry at `path`.
pub fn set_size(entries: &mut [treeidx::Entry], path: &str, new_size: u64) {
	let entry = entries
		.iter_mut()
		.find(|e| e.path == path.as_bytes())
		.expect("entry exists");
	match &mut entry.kind {
		treeidx::Kind::File {
			size,
			..
		} => *size = new_size,
		_ => panic!("{path} is not a file"),
	}
}

/// Rewrites a stored file's bytes in place (same inode), lifting its read-only mode for the write.
pub fn overwrite(p: &Path, body: &[u8]) {
	let mode = fs::metadata(p).expect("stat").permissions().mode();
	fs::set_permissions(p, fs::Permissions::from_mode(0o644)).expect("unlock");
	fs::write(p, body).expect("overwrite");
	fs::set_permissions(p, fs::Permissions::from_mode(mode)).expect("relock");
}

/// After a failed fetch: no tree, no index, no temps.
pub fn assert_nothing_committed(local: &repo::Repo, hash: &gittree::Hash) {
	assert!(!local.tree_path(hash).exists(), "no tree may be committed");
	assert!(!local.index_path(hash).exists(), "no index may be written");
	assert_eq!(leftover_temps(local), Vec::<std::path::PathBuf>::new());
}

pub fn blob_rel(idx: &treeidx::TreeIdx, path: &str) -> layout::RelPath {
	let (hash, exec) = blob_of(idx, path);
	layout::blob(&hash, exec)
}
