//! Integration tests for `gar::add`, exercised across its faith modes.
//!
//! The CLI exposes only one mode, so the per-mode contracts -- what each mode
//! shares or refuses to share with the user's original files -- live here,
//! against the library directly.

use std::fs;
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
