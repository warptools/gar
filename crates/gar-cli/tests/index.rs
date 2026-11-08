//! End-to-end tests for `gar index`.
//!
//! `gar add` always writes an index, so an unindexed tree is made by adding one and deleting its index file,
//! which is what a heap from before indexes looks like.
//! The exit codes and the stdout/stderr split are public interface, and pinned here.

mod common;

use common::{gar, scratch, treecas_entry, treeidx_entry};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

/// Lays down a one-file tree and adds it, returning the treehash.
fn add_tree(root: &Path, name: &str) -> String {
	let dir = root.join(name);
	fs::create_dir(&dir).expect("can make the tree dir");
	fs::write(dir.join("only.txt"), name).expect("can write a file");
	let r = gar(root, &["add", name]);
	assert_eq!(r.code, 0, "add {name}: stderr: {}", r.stderr);
	r.stdout.trim().to_owned()
}

/// Adds a tree, then removes its index file, returning the treehash and the index bytes add wrote.
fn add_unindexed_tree(root: &Path, name: &str) -> (String, Vec<u8>) {
	let hash = add_tree(root, name);
	let index = treeidx_entry(root, &hash);
	let bytes = fs::read(&index).expect("add wrote an index");
	fs::remove_file(&index).expect("can remove the index");
	(hash, bytes)
}

/// Overwrites a stored file's bytes in place (same inode); the read-only mode is lifted for the write.
fn corrupt_bytes(p: &Path) {
	fs::set_permissions(p, fs::Permissions::from_mode(0o644)).expect("can unlock the file");
	fs::write(p, "rot\n").expect("can overwrite the file");
	fs::set_permissions(p, fs::Permissions::from_mode(0o444)).expect("can relock the file");
}

/// Backfill lands the same bytes add wrote, and says nothing on stdout.
/// A second run skips the tree and leaves its index alone.
#[test]
fn index_named_restores_what_add_wrote() {
	let td = scratch("index-named");
	gar(td.path(), &["init"]);
	let (hash, add_bytes) = add_unindexed_tree(td.path(), "tree");

	let r = gar(td.path(), &["index", &hash]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert_eq!(
		r.stderr,
		"index: indexed 1 trees; 0 already indexed; 0 corrupt; 0 not in this heap\n"
	);
	let index = treeidx_entry(td.path(), &hash);
	assert_eq!(fs::read(&index).expect("index restored"), add_bytes);

	let r = gar(td.path(), &["index", &hash]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(
		r.stderr
			.starts_with("index: indexed 0 trees; 1 already indexed;"),
		"stderr: {}",
		r.stderr
	);
	assert_eq!(fs::read(&index).expect("index kept"), add_bytes);
}

/// `--all` fills only the trees that lack an index.
#[test]
fn index_all_fills_the_gap() {
	let td = scratch("index-all");
	gar(td.path(), &["init"]);
	let has = add_tree(td.path(), "has");
	let (lacks_a, bytes_a) = add_unindexed_tree(td.path(), "lacks_a");
	let (lacks_b, bytes_b) = add_unindexed_tree(td.path(), "lacks_b");

	let r = gar(td.path(), &["index", "--all"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert!(
		r.stderr
			.starts_with("index: indexed 2 trees; 1 already indexed;"),
		"stderr: {}",
		r.stderr
	);
	assert!(treeidx_entry(td.path(), &has).is_file());
	assert_eq!(
		fs::read(treeidx_entry(td.path(), &lacks_a)).expect("read"),
		bytes_a
	);
	assert_eq!(
		fs::read(treeidx_entry(td.path(), &lacks_b)).expect("read"),
		bytes_b
	);
}

/// A corrupt tree fails the run, is named in the diagnostic with a pointer at repair, and gets no index;
/// the other tree is still indexed.
#[test]
fn index_refuses_a_corrupt_tree_and_finishes_the_rest() {
	let td = scratch("index-corrupt");
	gar(td.path(), &["init"]);
	let (sound, _) = add_unindexed_tree(td.path(), "sound");
	let (rotten, _) = add_unindexed_tree(td.path(), "rotten");
	corrupt_bytes(&treecas_entry(td.path(), &rotten).join("only.txt"));

	let r = gar(td.path(), &["index", "--all"]);
	assert_eq!(r.code, 10, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert!(
		r.stderr
			.starts_with("index: indexed 1 trees; 0 already indexed; 1 corrupt;"),
		"stderr: {}",
		r.stderr
	);
	let line = r
		.stderr
		.lines()
		.find(|l| l.contains(&rotten))
		.expect("a diagnostic names the corrupt tree");
	assert!(
		line.contains("corrupt") && line.contains("gar repair"),
		"{line}"
	);
	assert!(!treeidx_entry(td.path(), &rotten).exists());
	assert!(treeidx_entry(td.path(), &sound).is_file());
}

/// A named tree that isn't in the heap fails the run, and the other named tree is still indexed.
#[test]
fn index_reports_a_named_tree_not_in_stock() {
	let td = scratch("index-not-in-stock");
	gar(td.path(), &["init"]);
	let (present, _) = add_unindexed_tree(td.path(), "present");
	let absent = "f".repeat(64);

	let r = gar(td.path(), &["index", &absent, &present]);
	assert_eq!(r.code, 6, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert!(
		r.stderr
			.contains(&format!("tree {absent} is not in this heap")),
		"stderr: {}",
		r.stderr
	);
	assert!(treeidx_entry(td.path(), &present).is_file());
}

/// With both failures in one run, corruption decides the exit code.
#[test]
fn corruption_outranks_not_in_stock() {
	let td = scratch("index-both-failures");
	gar(td.path(), &["init"]);
	let (rotten, _) = add_unindexed_tree(td.path(), "rotten");
	corrupt_bytes(&treecas_entry(td.path(), &rotten).join("only.txt"));

	let r = gar(td.path(), &["index", &"f".repeat(64), &rotten]);
	assert_eq!(r.code, 10, "stderr: {}", r.stderr);
	assert!(
		r.stderr.starts_with(
			"index: indexed 0 trees; 0 already indexed; 1 corrupt; 1 not in this heap\n"
		),
		"stderr: {}",
		r.stderr
	);
}

/// Which trees to index must be said: no selection, both selections, and a malformed hash are usage errors.
/// None of them touch the heap.
#[test]
fn index_usage_errors() {
	let td = scratch("index-usage");
	gar(td.path(), &["init"]);
	let (hash, _) = add_unindexed_tree(td.path(), "tree");
	let upper = hash.to_uppercase();
	for args in [
		vec!["index"],
		vec!["index", "--all", &hash],
		vec!["index", &upper],
	] {
		let r = gar(td.path(), &args);
		assert_eq!(r.code, 1, "{args:?}: stderr: {}", r.stderr);
		assert!(r.stdout.is_empty(), "{args:?}: stdout: {}", r.stdout);
	}
	assert!(!treeidx_entry(td.path(), &hash).exists());
}

/// A heap from before indexes has no treeidx dir at all; index makes it.
#[test]
fn index_restores_a_missing_treeidx_dir() {
	let td = scratch("index-no-treeidx-dir");
	gar(td.path(), &["init"]);
	let (hash, add_bytes) = add_unindexed_tree(td.path(), "tree");
	fs::remove_dir_all(td.path().join(".gar/treeidx")).expect("can remove the treeidx dir");

	let r = gar(td.path(), &["index", "--all"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert_eq!(
		fs::read(treeidx_entry(td.path(), &hash)).expect("read"),
		add_bytes
	);
}
