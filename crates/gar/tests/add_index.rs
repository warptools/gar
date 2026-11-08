//! Integration tests for the treeidx file `gar::add` writes for every tree it stores.
//!
//! The cross-check throughout is `treeidx::generate`, which indexes a tree by walking
//! and re-hashing it from disk: add's index is built from its own walk instead,
//! so agreement between the two means add's bookkeeping describes what actually landed.

use std::fs;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use gar::add::{self, FaithMode, Indexing};
use gar::repo;
use gar::treeidx;
use rstest::rstest;

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

fn manifest_dir() -> PathBuf {
	PathBuf::from(
		std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for tests"),
	)
}

/// Plants a tree with every entry kind an index records, at `root/specimen`:
/// nested and empty dirs, an executable, a symlink, and names with a space and a newline in them.
fn specimen(root: &Path) -> PathBuf {
	let dir = root.join("specimen");
	fs::create_dir_all(dir.join("nested/empty")).expect("mkdir");
	fs::create_dir(dir.join("empty")).expect("mkdir");
	fs::write(dir.join("a_file"), b"a file\n").expect("write");
	fs::write(dir.join("has space"), b"spaced\n").expect("write");
	fs::write(dir.join("nested/new\nline"), b"newlined\n").expect("write");
	fs::write(dir.join("nested/run.sh"), b"#!/bin/sh\n").expect("write");
	fs::set_permissions(dir.join("nested/run.sh"), fs::Permissions::from_mode(0o755))
		.expect("chmod");
	std::os::unix::fs::symlink("nested/run.sh", dir.join("link")).expect("symlink");
	dir
}

/// Asserts the heap holds an index for `hash` that is exactly what an independent disk walk
/// of the stored tree produces, and that it parses back to the same root hash.
fn assert_index_matches_stored_tree(heap: &repo::Repo, hash: &gittree::Hash) {
	let bytes = fs::read(heap.index_path(hash)).expect("the index file is readable");
	let walked = treeidx::generate(&heap.tree_path(hash)).expect("the stored tree indexes");
	assert_eq!(
		String::from_utf8_lossy(&bytes),
		String::from_utf8_lossy(&treeidx::serialize(&walked)),
		"add's index should be byte-identical to a fresh walk of the stored tree"
	);
	let parsed = treeidx::parse(&bytes).expect("the index parses");
	assert_eq!(parsed.root_hash(), hash);
}

/// Index files land read-only, like blobs.
/// One test covers every writer (add, import, backfill, fetch): they share the one landing path.
#[test]
fn index_lands_read_only() {
	let td = scratch("index-ro");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let dir = specimen(td.path());
	let hash = add::add(&heap, &dir, FaithMode::Copy).expect("add");
	let mode = fs::metadata(heap.index_path(&hash))
		.expect("the index file exists")
		.permissions()
		.mode();
	assert_eq!(
		mode & 0o7777,
		repo::INDEX_MODE,
		"index files land mode 0444, umask notwithstanding"
	);
}

#[rstest]
#[case::copy(FaithMode::Copy)]
#[case::link_originals(FaithMode::LinkOriginals)]
fn add_writes_an_index_matching_the_stored_tree(#[case] mode: FaithMode) {
	let td = scratch("index-specimen");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let dir = specimen(td.path());
	let hash = add::add(&heap, &dir, mode).expect("add");
	assert_index_matches_stored_tree(&heap, &hash);
}

/// `gar add .` with the heap inside the tree: add skips the heap's own directory,
/// so the index must leave it out too.
#[rstest]
#[case::copy(FaithMode::Copy)]
#[case::link_originals(FaithMode::LinkOriginals)]
fn index_leaves_out_a_heap_inside_the_tree(#[case] mode: FaithMode) {
	let td = scratch("index-heap-inside");
	let dir = specimen(td.path());
	let heap = repo::init_at(&dir).expect("creating a repo inside the specimen");
	let hash = add::add(&heap, &dir, mode).expect("add");
	assert_index_matches_stored_tree(&heap, &hash);
	let idx = treeidx::parse(&fs::read(heap.index_path(&hash)).expect("read")).expect("parse");
	assert!(
		idx.entries().iter().all(|e| !e.path.starts_with(b".gar")),
		"the heap's own directory should not be indexed"
	);
}

/// The index file's location is an external contract (static servers hand it out by path),
/// so it's spelled out literally here; and its bytes are pinned by the hand-written golden.
#[test]
fn alpha_indexes_to_the_golden_bytes() {
	let td = scratch("index-alpha");
	let heap = repo::init_at(td.path()).expect("creating a repo");
	// Copy mode: the fixture is in the source checkout, likely on another filesystem than the tempdir.
	let alpha = manifest_dir().join("../gittree/fixtures/alpha");
	let hash = add::add(&heap, &alpha, FaithMode::Copy).expect("add alpha");
	let hex = "9024a7f8afa43db06ff2b50d9ac9c21b791bee49d8092d3f14f1e433bfd927fa";
	assert_eq!(hash.as_hex(), hex);
	let bytes = fs::read(td.path().join(format!(".gar/treeidx/90/24/{hex}")))
		.expect("the index is at its sharded path");
	let golden = fs::read(manifest_dir().join("fixtures/treeidx/alpha.treeidx"))
		.expect("the golden fixture is readable");
	assert_eq!(
		String::from_utf8_lossy(&bytes),
		String::from_utf8_lossy(&golden)
	);
}

/// Re-adding a stored tree loses the treecas rename to the existing entry, which is success;
/// the index still gets written on that path, so a re-add is also how a missing index comes back.
#[test]
fn readding_keeps_and_restores_the_index() {
	let td = scratch("index-readd");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let dir = specimen(td.path());
	let hash = add::add(&heap, &dir, FaithMode::Copy).expect("first add");
	let index = heap.index_path(&hash);
	let first = fs::read(&index).expect("first add wrote an index");

	add::add(&heap, &dir, FaithMode::Copy).expect("re-add over a present index");
	assert_eq!(fs::read(&index).expect("index still present"), first);

	// Remove the index and both shard levels above it: the re-add has to make them again.
	fs::remove_file(&index).expect("remove the index");
	let inner = index.parent().expect("index sits in a shard dir");
	fs::remove_dir(inner).expect("remove the inner shard dir");
	fs::remove_dir(inner.parent().expect("two shard levels")).expect("remove the outer shard dir");
	add::add(&heap, &dir, FaithMode::Copy).expect("re-add with the index gone");
	assert_eq!(fs::read(&index).expect("index restored"), first);
}

/// With indexing switched off, the treeidx dir stays empty and everything else is as usual.
#[rstest]
#[case::copy(FaithMode::Copy)]
#[case::link_originals(FaithMode::LinkOriginals)]
fn skip_writes_no_index(#[case] mode: FaithMode) {
	let td = scratch("index-skip");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let dir = specimen(td.path());
	let want = gittree::hash_of_path(&dir).expect("hash the specimen");
	let hash = add::add_with(&heap, &dir, mode, Indexing::Skip).expect("add");
	assert_eq!(hash, want);
	assert_eq!(
		gittree::hash_of_path(heap.tree_path(&hash)).expect("hash the stored tree"),
		want
	);
	let leftovers: Vec<_> = fs::read_dir(heap.treeidx_path())
		.expect("read the treeidx dir")
		.map(|e| e.expect("readable dirent").file_name())
		.collect();
	assert!(leftovers.is_empty(), "treeidx dir has: {leftovers:?}");
}

/// The index lands only after the tree does: an add whose treecas commit fails leaves no index
/// claiming a tree the heap doesn't hold.
#[test]
fn a_failed_commit_leaves_no_index() {
	let td = scratch("index-failed-commit");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let dir = specimen(td.path());
	let hash = gittree::hash_of_path(&dir).expect("hash the specimen");
	// A plain file squatting at the tree's name makes the commit fail.
	heap.create_tree_shard(&hash).expect("make the tree shard");
	fs::write(heap.tree_path(&hash), b"not a tree\n").expect("plant the imposter");

	add::add(&heap, &dir, FaithMode::Copy).expect_err("add onto an imposter must fail");
	assert!(
		!heap.index_path(&hash).exists(),
		"no index may exist for a tree that didn't land"
	);
}

/// Names that aren't UTF-8 go through as raw bytes.
#[test]
fn non_utf8_names_are_indexed_as_bytes() {
	let td = scratch("index-non-utf8");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let dir = td.path().join("bytes");
	fs::create_dir_all(dir.join(std::ffi::OsStr::from_bytes(b"dir \xff"))).expect("mkdir");
	fs::write(
		dir.join(std::ffi::OsStr::from_bytes(b"dir \xff/file \xfe")),
		b"bytes\n",
	)
	.expect("write");
	let hash = add::add(&heap, &dir, FaithMode::Copy).expect("add");
	assert_index_matches_stored_tree(&heap, &hash);
}
