//! Integration tests for backfilling index files into a heap (`gar::index_backfill`).
//!
//! Unindexed trees come from `add_with(.., Indexing::Skip)`, which stores a tree exactly as add does, minus the index.

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use gar::add::{self, FaithMode, Indexing};
use gar::index_backfill::{self, CorruptTree};
use gar::repo;
use gar::treeidx;

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// Plants a tree with every entry kind an index records, at `root/{name}`:
/// nested and empty dirs, an executable, a symlink, and names with a space and a newline in them.
/// `flavor` lands in one file, so differently flavored specimens are different trees.
fn specimen(root: &Path, name: &str, flavor: &str) -> PathBuf {
	let dir = root.join(name);
	fs::create_dir_all(dir.join("nested/empty")).expect("mkdir");
	fs::create_dir(dir.join("empty")).expect("mkdir");
	fs::write(dir.join("a_file"), flavor).expect("write");
	fs::write(dir.join("has space"), b"spaced\n").expect("write");
	fs::write(dir.join("nested/new\nline"), b"newlined\n").expect("write");
	fs::write(dir.join("nested/run.sh"), b"#!/bin/sh\n").expect("write");
	fs::set_permissions(dir.join("nested/run.sh"), fs::Permissions::from_mode(0o755))
		.expect("chmod");
	std::os::unix::fs::symlink("nested/run.sh", dir.join("link")).expect("symlink");
	dir
}

/// Stores a fresh specimen in `heap`, with or without its index, and returns its treehash.
fn store(heap: &repo::Repo, root: &Path, flavor: &str, indexing: Indexing) -> gittree::Hash {
	let dir = specimen(root, flavor, flavor);
	add::add_with(heap, &dir, FaithMode::Copy, indexing).expect("add")
}

/// Overwrites a stored file's bytes in place (same inode), as disk rot would.
/// Stored files are read-only blobs, so the mode is lifted for the write and put back after.
fn corrupt_bytes(p: &Path) {
	let mode = fs::metadata(p).expect("stat").permissions().mode();
	fs::set_permissions(p, fs::Permissions::from_mode(0o644)).expect("unlock");
	fs::write(p, b"rot\n").expect("overwrite");
	fs::set_permissions(p, fs::Permissions::from_mode(mode)).expect("relock");
}

fn ino(p: &Path) -> u64 {
	fs::metadata(p).expect("stat").ino()
}

/// The repo dir's own entries, sorted; a leftover temp dir would show up here.
fn repo_dir_entries(heap: &repo::Repo) -> Vec<String> {
	let mut names: Vec<String> = fs::read_dir(heap.repo_path())
		.expect("read the repo dir")
		.map(|e| e.expect("dirent").file_name().into_string().expect("utf8"))
		.collect();
	names.sort();
	names
}

/// The backfilled index is byte-identical to the one add writes for the same tree.
/// Add builds its index from its own walk, and backfill from a fresh disk walk of the stored tree,
/// so this pins the two paths against each other.
#[test]
fn backfill_writes_what_add_would_have() {
	let td = scratch("backfill-like-add");
	let skipped = repo::init_at(td.path().join("skipped")).expect("creating a repo");
	let indexed = repo::init_at(td.path().join("indexed")).expect("creating a repo");
	let hash = store(&skipped, td.path(), "same", Indexing::Skip);
	assert!(!skipped.index_path(&hash).exists());
	let same = add::add(&indexed, td.path().join("same"), FaithMode::Copy).expect("add");
	assert_eq!(same, hash);
	let want = fs::read(indexed.index_path(&hash)).expect("add wrote an index");

	let report = index_backfill::index_named(&skipped, &[hash.clone()]).expect("backfill");
	assert_eq!((report.indexed, report.already_indexed), (1, 0));
	assert!(report.corrupt.is_empty() && report.not_in_stock.is_empty());
	let got = fs::read(skipped.index_path(&hash)).expect("backfill wrote an index");
	assert_eq!(
		String::from_utf8_lossy(&got),
		String::from_utf8_lossy(&want)
	);
	assert_eq!(treeidx::parse(&got).expect("parses").root_hash(), &hash);
	assert_eq!(
		repo_dir_entries(&skipped),
		["blobcas", "treecas", "treeidx"]
	);
}

/// A second run finds the index present and leaves the file alone:
/// same bytes, and the same inode (a rewrite would land a new one by rename).
#[test]
fn second_run_skips() {
	let td = scratch("backfill-idempotent");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let hash = store(&heap, td.path(), "once", Indexing::Skip);
	index_backfill::index_named(&heap, &[hash.clone()]).expect("first backfill");
	let index = heap.index_path(&hash);
	let (bytes, inode) = (fs::read(&index).expect("read"), ino(&index));

	let report = index_backfill::index_named(&heap, &[hash.clone()]).expect("second backfill");
	assert_eq!((report.indexed, report.already_indexed), (0, 1));
	assert_eq!(fs::read(&index).expect("read"), bytes);
	assert_eq!(ino(&index), inode);

	let report = index_backfill::index_all(&heap).expect("backfill all");
	assert_eq!((report.indexed, report.already_indexed), (0, 1));
	assert_eq!(ino(&index), inode);
}

/// `index_all` on a heap with indexed and unindexed trees fills only the gap.
/// Entries the treecas walk doesn't recognize as trees are left alone.
#[test]
fn index_all_fills_only_the_gap() {
	let td = scratch("backfill-all");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let has = store(&heap, td.path(), "has", Indexing::Write);
	let lacks_a = store(&heap, td.path(), "lacks_a", Indexing::Skip);
	let lacks_b = store(&heap, td.path(), "lacks_b", Indexing::Skip);
	let had_inode = ino(&heap.index_path(&has));
	// Not trees: an uppercase name, a dot-temp like an in-flight add leaves, and a plain file with a hash name.
	let shard = heap.tree_path(&has).parent().expect("shard dir").to_owned();
	fs::create_dir(shard.join(has.as_hex().to_uppercase())).expect("mkdir");
	fs::create_dir(shard.join(".wiptree-xyz")).expect("mkdir");
	fs::write(shard.join("0".repeat(64)), b"not a tree\n").expect("write");

	let report = index_backfill::index_all(&heap).expect("backfill all");
	assert_eq!((report.indexed, report.already_indexed), (2, 1));
	assert!(report.corrupt.is_empty() && report.not_in_stock.is_empty());
	assert_eq!(ino(&heap.index_path(&has)), had_inode);
	for hash in [&lacks_a, &lacks_b] {
		let got = fs::read(heap.index_path(hash)).expect("backfilled");
		let walked = treeidx::generate(&heap.tree_path(hash)).expect("indexes");
		assert_eq!(got, treeidx::serialize(&walked));
	}
}

/// A tree whose content no longer hashes to its name gets no index, and is reported
/// with the hash it does recompute to; the other trees are still indexed.
/// Two corrupt trees, so the report's order is pinned too.
#[test]
fn corrupt_trees_get_no_index_and_the_rest_go_on() {
	let td = scratch("backfill-corrupt");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let sound = store(&heap, td.path(), "sound", Indexing::Skip);
	let mut want: Vec<CorruptTree> = ["rotten1", "rotten2"]
		.iter()
		.map(|flavor| {
			let name = store(&heap, td.path(), flavor, Indexing::Skip);
			// `a_file` holds the flavor, so its blob is this tree's alone; other files share blobs with `sound`.
			corrupt_bytes(&heap.tree_path(&name).join("a_file"));
			let computed = gittree::hash_of_path(heap.tree_path(&name)).expect("hash");
			assert_ne!(computed, name, "the vandalism should be observable");
			CorruptTree {
				name,
				computed,
			}
		})
		.collect();
	want.sort_by_key(|c| c.name.as_hex());

	let report = index_backfill::index_all(&heap).expect("backfill all");
	assert_eq!(report.corrupt, want);
	assert_eq!((report.indexed, report.already_indexed), (1, 0));
	assert!(heap.index_path(&sound).is_file());
	for c in &want {
		assert!(!heap.index_path(&c.name).exists());
	}

	// Named (in reverse hex order), it's the same story; and still no index on a second try.
	let named = [want[1].name.clone(), sound.clone(), want[0].name.clone()];
	let report = index_backfill::index_named(&heap, &named).expect("backfill");
	assert_eq!(report.corrupt, want);
	assert_eq!((report.indexed, report.already_indexed), (0, 1));
	for c in &want {
		assert!(!heap.index_path(&c.name).exists());
	}
}

/// A present index means the tree is skipped without being read:
/// even a tree that's gone corrupt underneath its index is counted as already indexed.
/// Checking indexes against their trees isn't backfill's job, and re-hashing every indexed tree would make re-runs slow.
#[test]
fn a_present_index_is_not_rechecked() {
	let td = scratch("backfill-no-recheck");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let hash = store(&heap, td.path(), "indexed", Indexing::Write);
	corrupt_bytes(&heap.tree_path(&hash).join("a_file"));

	let report = index_backfill::index_all(&heap).expect("backfill all");
	assert_eq!((report.indexed, report.already_indexed), (0, 1));
	assert!(report.corrupt.is_empty());
}

/// Named trees the heap doesn't hold are reported, in hex order, and the rest are still indexed.
#[test]
fn named_trees_not_in_stock_are_reported() {
	let td = scratch("backfill-not-in-stock");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let hash = store(&heap, td.path(), "present", Indexing::Skip);
	let absent_hi = gittree::Hash::from_hex([b'f'; 64]).expect("hex");
	let absent_lo = gittree::Hash::from_hex([b'1'; 64]).expect("hex");

	let report =
		index_backfill::index_named(&heap, &[absent_hi.clone(), hash.clone(), absent_lo.clone()])
			.expect("backfill");
	assert_eq!(report.not_in_stock, [absent_lo, absent_hi]);
	assert_eq!((report.indexed, report.already_indexed), (1, 0));
	assert!(heap.index_path(&hash).is_file());
}

/// A landing that fails cleans up after itself: no temp dir is left in the repo dir.
#[test]
fn a_failed_landing_leaves_no_temp() {
	let td = scratch("backfill-failed-landing");
	let heap = repo::init_at(td.path().join("heap")).expect("creating a repo");
	let hash = store(&heap, td.path(), "orphaned", Indexing::Skip);
	// Shard creation refuses to make a missing CAS root, so the landing fails.
	fs::remove_dir(heap.treeidx_path()).expect("remove the treeidx root");

	let err =
		index_backfill::index_named(&heap, &[hash]).expect_err("landing without a treeidx root");
	assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
	assert_eq!(repo_dir_entries(&heap), ["blobcas", "treecas"]);
}
