//! Integration tests for `gar::add`, exercised across its faith modes.
//!
//! The CLI exposes only one mode, so the per-mode contracts -- what each mode
//! shares or refuses to share with the user's original files -- live here,
//! against the library directly.

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use gar::add::{self, FaithMode};
use gar::repo;
use rstest::rstest;

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// Makes a one-file tree at `root/stuff` and returns its path.  The file says `a file\n`.
fn small_tree(root: &Path) -> PathBuf {
	let stuff = root.join("stuff");
	fs::create_dir(&stuff).expect("mkdir stuff");
	fs::write(stuff.join("a_file"), b"a file\n").expect("write a file");
	stuff
}

/// The git blob hash of `a file\n`, for locating that content in the blobcas.
fn a_file_blob_hash() -> gittree::Hash {
	gittree::hash_of_stream(&mut &b"a file\n"[..], 7).expect("hashing a slice cannot io-fail")
}

/// A shard dir can go missing after a repo is in use (partial restores, overzealous cleanup);
/// the next add that needs it should recreate it rather than report the hole as an error.
#[rstest]
#[case::copy(FaithMode::Copy)]
#[case::link_originals(FaithMode::LinkOriginals)]
fn a_removed_shard_dir_is_recreated_on_the_next_add(#[case] mode: FaithMode) {
	let td = scratch("shard-recreate");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let stuff = small_tree(td.path());

	let h1 = add::add(&heap, &stuff, mode).expect("first add");
	let blob = heap.blob_path(&a_file_blob_hash(), false);
	assert!(blob.is_file(), "the first add should have stored the blob");

	// Remove the blob and both shard levels above it.
	fs::remove_file(&blob).expect("remove the blob");
	let inner = blob.parent().expect("blob sits in a shard dir");
	let outer = inner.parent().expect("shard dirs come in two levels");
	fs::remove_dir(inner).expect("remove the inner shard dir");
	fs::remove_dir(outer).expect("remove the outer shard dir");

	let h2 = add::add(&heap, &stuff, mode).expect("add after the shard dir went missing");
	assert_eq!(h1, h2);
	assert!(blob.is_file(), "the blob should have been stored again");
}

/// The heart of each mode's contract: copy mode leaves the heap with its own private inodes,
/// link mode deliberately shares the original's.
#[rstest]
#[case::copy(FaithMode::Copy, false)]
#[case::link_originals(FaithMode::LinkOriginals, true)]
fn blob_inode_sharing_with_the_original_is_per_mode(
	#[case] mode: FaithMode,
	#[case] expect_shared: bool,
) {
	let td = scratch("inode-sharing");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let stuff = small_tree(td.path());

	add::add(&heap, &stuff, mode).expect("add");
	let original_ino = fs::metadata(stuff.join("a_file"))
		.expect("stat original")
		.ino();
	let blob_ino = fs::metadata(heap.blob_path(&a_file_blob_hash(), false))
		.expect("stat blob")
		.ino();
	assert_eq!(blob_ino == original_ino, expect_shared);
}

/// The reason copy is the default: mutating an original after `add` must not reach the heap.
#[test]
fn overwriting_an_original_after_add_leaves_the_heap_intact() {
	let td = scratch("overwrite-original");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let stuff = small_tree(td.path());

	let hash = add::add(&heap, &stuff, FaithMode::Copy).expect("add");
	fs::write(stuff.join("a_file"), b"vandalism\n").expect("overwrite the original in place");

	let blob = heap.blob_path(&a_file_blob_hash(), false);
	assert_eq!(fs::read(&blob).expect("read blob"), b"a file\n");
	let treecas_file = heap.tree_path(&hash).join("a_file");
	assert_eq!(
		fs::read(&treecas_file).expect("read treecas file"),
		b"a file\n"
	);
}

/// Dedup must survive the detour through staging: a re-add links to the blob that's
/// already there (same inode, not a lookalike), and staging leaves nothing behind.
#[test]
fn readding_reuses_blob_inodes_and_leaves_no_staging() {
	let td = scratch("readd-dedup");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let stuff = small_tree(td.path());

	let h1 = add::add(&heap, &stuff, FaithMode::Copy).expect("first add");
	let blob = heap.blob_path(&a_file_blob_hash(), false);
	let ino1 = fs::metadata(&blob).expect("stat blob").ino();

	// A second tree sharing the same file content, so blob placement runs again
	// (a plain re-add of the same tree would also work, but this shows dedup across trees).
	let other = td.path().join("other");
	fs::create_dir(&other).expect("mkdir other");
	fs::write(other.join("same_content"), b"a file\n").expect("write a file");
	let h2 = add::add(&heap, &other, FaithMode::Copy).expect("second add");
	assert_ne!(h1, h2, "different trees, different tree hashes");

	assert_eq!(
		fs::metadata(&blob).expect("stat blob").ino(),
		ino1,
		"the blob should still be the same inode, not a replacement"
	);
	assert_eq!(
		fs::metadata(heap.tree_path(&h2).join("same_content"))
			.expect("stat the new treecas file")
			.ino(),
		ino1,
		"the new treecas entry should share the deduplicated blob's inode"
	);
	let leftovers: Vec<_> = fs::read_dir(heap.repo_path())
		.expect("read the repo dir")
		.map(|e| e.expect("readable dirent").file_name())
		.filter(|n| n.to_string_lossy().starts_with(".staging-"))
		.collect();
	assert!(
		leftovers.is_empty(),
		"staging dirs left behind: {leftovers:?}"
	);
}

/// Blob modes are a format guarantee (see README_formats.md): read-only,
/// 0444 or 0555 per the `-x` split, no matter how the source was dressed.
#[test]
fn copied_blob_permissions_are_normalized() {
	let td = scratch("blob-perms");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let stuff = td.path().join("stuff");
	fs::create_dir(&stuff).expect("mkdir stuff");
	fs::write(stuff.join("a_file"), b"a file\n").expect("write a file");
	fs::set_permissions(stuff.join("a_file"), fs::Permissions::from_mode(0o600))
		.expect("chmod the file to something private");
	fs::write(stuff.join("a_script"), b"a file\n").expect("write a script");
	fs::set_permissions(stuff.join("a_script"), fs::Permissions::from_mode(0o700))
		.expect("chmod the script to something private");

	add::add(&heap, &stuff, FaithMode::Copy).expect("add");
	let mode_of = |executable| {
		fs::metadata(heap.blob_path(&a_file_blob_hash(), executable))
			.expect("stat blob")
			.permissions()
			.mode() & 0o7777
	};
	assert_eq!(mode_of(false), 0o444);
	assert_eq!(mode_of(true), 0o555);
}
