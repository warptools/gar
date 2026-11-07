//! End-to-end tests for `gar gc`.
//!
//! Gc is the one command that destroys data, so these tests pin both directions hard:
//! what a demands list keeps (bit-for-bit, hash-verifiable), and what it removes.
//! The demands file format and the exit codes are public interface; they're pinned here too.

mod common;

use common::{arg, blobcas_entry, gar, scratch, treecas_entry};
use std::path::Path;

/// Lays down a directory of files under `root` and adds it to the heap, returning the treehash.
fn add_tree(root: &Path, name: &str, files: &[(&str, &str)]) -> String {
	let dir = root.join(name);
	std::fs::create_dir(&dir).expect("can make the tree dir");
	for (file, content) in files {
		std::fs::write(dir.join(file), content).expect("can write a file");
	}
	let r = gar(root, &["add", name]);
	assert_eq!(r.code, 0, "add {name}: stderr: {}", r.stderr);
	r.stdout.trim().to_owned()
}

/// The blob hash of a loose file, via `gar hash` (which hashes files as git blobs).
fn blob_hash_of(root: &Path, path: &str) -> String {
	let r = gar(root, &["hash", path]);
	assert_eq!(r.code, 0, "hash {path}: stderr: {}", r.stderr);
	r.stdout.trim().to_owned()
}

fn write_demands(root: &Path, name: &str, lines: &[&str]) -> String {
	let mut text = String::new();
	for line in lines {
		text.push_str(line);
		text.push('\n');
	}
	std::fs::write(root.join(name), text).expect("can write the demands file");
	name.to_owned()
}

/// Every regular file in the blobcas, as paths relative to it.  Empty shard dirs don't count.
fn blobcas_files(root: &Path) -> Vec<std::path::PathBuf> {
	fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
		for ent in std::fs::read_dir(dir).expect("can read the blobcas") {
			let ent = ent.expect("readable dirent");
			if ent.file_type().expect("has a file type").is_dir() {
				walk(&ent.path(), out);
			} else {
				out.push(ent.path());
			}
		}
	}
	let mut out = Vec::new();
	walk(&root.join(".gar/blobcas"), &mut out);
	out
}

/// The demanded tree survives intact (its hash still verifies), the other is gone.
/// The demands file also exercises the comment and blank-line allowances of the format.
#[test]
fn keep_what_is_demanded() {
	let td = scratch("gc-keep");
	gar(td.path(), &["init"]);
	let kept = add_tree(td.path(), "kept", &[("a.txt", "aaa\n")]);
	let doomed = add_tree(td.path(), "doomed", &[("b.txt", "bbb\n")]);
	let demands = write_demands(td.path(), "keep.txt", &["# kept by the test", "", &kept]);

	let r = gar(td.path(), &["gc", "--demands", &demands]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(
		r.stdout.is_empty(),
		"gc chatter belongs on stderr; stdout: {}",
		r.stdout
	);
	assert!(!treecas_entry(td.path(), &doomed).exists());
	// Intact means hash-verifiable, not merely present.
	let rehash = gar(td.path(), &["hash", arg(&treecas_entry(td.path(), &kept))]);
	assert_eq!(rehash.stdout.trim(), kept);
}

/// A blob shared between a kept and a removed tree survives; blobs unique to the removed tree go.
#[test]
fn shared_blobs_survive() {
	let td = scratch("gc-shared-blobs");
	gar(td.path(), &["init"]);
	let kept = add_tree(
		td.path(),
		"kept",
		&[("shared.txt", "shared\n"), ("mine.txt", "mine\n")],
	);
	add_tree(
		td.path(),
		"doomed",
		&[("shared.txt", "shared\n"), ("theirs.txt", "theirs\n")],
	);
	let shared_blob = blob_hash_of(td.path(), "kept/shared.txt");
	let doomed_blob = blob_hash_of(td.path(), "doomed/theirs.txt");
	let demands = write_demands(td.path(), "keep.txt", &[&kept]);

	let r = gar(td.path(), &["gc", "--demands", &demands]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(
		blobcas_entry(td.path(), &shared_blob).exists(),
		"the shared blob must survive"
	);
	assert!(
		!blobcas_entry(td.path(), &doomed_blob).exists(),
		"the removed tree's unique blob must be gone"
	);
	let content = std::fs::read_to_string(treecas_entry(td.path(), &kept).join("shared.txt"))
		.expect("the kept tree's file still reads");
	assert_eq!(content, "shared\n");
}

/// Demanding nothing, explicitly, clears the heap.  Shard dirs may remain; content may not.
#[test]
fn empty_demands_clears() {
	let td = scratch("gc-empty-demands");
	gar(td.path(), &["init"]);
	let tree = add_tree(td.path(), "stuff", &[("a.txt", "aaa\n")]);
	let demands = write_demands(td.path(), "keep.txt", &[]);

	let r = gar(td.path(), &["gc", "--demands", &demands]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(!treecas_entry(td.path(), &tree).exists());
	assert_eq!(blobcas_files(td.path()), Vec::<std::path::PathBuf>::new());
}

/// The kept set is the union of all the demands files given.
#[test]
fn multiple_demands_files_union() {
	let td = scratch("gc-union");
	gar(td.path(), &["init"]);
	let one = add_tree(td.path(), "one", &[("a.txt", "aaa\n")]);
	let two = add_tree(td.path(), "two", &[("b.txt", "bbb\n")]);
	let doomed = add_tree(td.path(), "doomed", &[("c.txt", "ccc\n")]);
	let d1 = write_demands(td.path(), "keep1.txt", &[&one]);
	let d2 = write_demands(td.path(), "keep2.txt", &[&two]);

	let r = gar(td.path(), &["gc", "--demands", &d1, "--demands", &d2]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(treecas_entry(td.path(), &one).is_dir());
	assert!(treecas_entry(td.path(), &two).is_dir());
	assert!(!treecas_entry(td.path(), &doomed).exists());
}

/// A demanded tree the heap doesn't hold is a failing exit --
/// someone's expectation is already broken -- but the gc still completes for everything else.
#[test]
fn demanded_but_absent_fails_after_completing() {
	let td = scratch("gc-absent-demand");
	gar(td.path(), &["init"]);
	let kept = add_tree(td.path(), "kept", &[("a.txt", "aaa\n")]);
	let doomed = add_tree(td.path(), "doomed", &[("b.txt", "bbb\n")]);
	let absent = "0000000000000000000000000000000000000000000000000000000000000000";
	let demands = write_demands(td.path(), "keep.txt", &[&kept, absent]);

	let r = gar(td.path(), &["gc", "--demands", &demands]);
	assert_eq!(r.code, 7, "stdout: {}, stderr: {}", r.stdout, r.stderr);
	assert!(
		r.stderr.contains(absent),
		"the report should name the absent hash; stderr: {}",
		r.stderr
	);
	assert!(treecas_entry(td.path(), &kept).is_dir());
	assert!(
		!treecas_entry(td.path(), &doomed).exists(),
		"the rest of the gc should still have happened"
	);
}

/// A single malformed line means nothing at all gets deleted: a gc driven by
/// a half-understood demands list must not get to touch the heap.
#[test]
fn malformed_demands_is_inert() {
	let td = scratch("gc-malformed");
	gar(td.path(), &["init"]);
	let one = add_tree(td.path(), "one", &[("a.txt", "aaa\n")]);
	let two = add_tree(td.path(), "two", &[("b.txt", "bbb\n")]);
	let blob = blob_hash_of(td.path(), "one/a.txt");
	let demands = write_demands(td.path(), "keep.txt", &[&one, "this is not a treehash"]);

	let r = gar(td.path(), &["gc", "--demands", &demands]);
	assert_eq!(r.code, 1, "stdout: {}, stderr: {}", r.stdout, r.stderr);
	assert!(
		r.stderr.contains("keep.txt") && r.stderr.contains("line 2"),
		"the diagnostic should name the file and line; stderr: {}",
		r.stderr
	);
	assert!(treecas_entry(td.path(), &one).is_dir());
	assert!(treecas_entry(td.path(), &two).is_dir());
	assert!(blobcas_entry(td.path(), &blob).exists());
}

/// Bare `gar gc` deleting everything is a footgun; demanding nothing takes an explicit empty file.
#[test]
fn gc_without_demands_is_refused() {
	let td = scratch("gc-no-demands");
	gar(td.path(), &["init"]);
	let tree = add_tree(td.path(), "stuff", &[("a.txt", "aaa\n")]);

	let r = gar(td.path(), &["gc"]);
	assert_eq!(r.code, 1, "stdout: {}, stderr: {}", r.stdout, r.stderr);
	assert!(treecas_entry(td.path(), &tree).is_dir());
}

/// An in-flight add's staging dir in the treecas root is never gc's to touch.
#[test]
fn staging_dirs_survive() {
	let td = scratch("gc-staging");
	gar(td.path(), &["init"]);
	add_tree(td.path(), "stuff", &[("a.txt", "aaa\n")]);
	// A wiptree mid-add holds arbitrary user content.  The planted content here is shaped
	// exactly like shard dirs holding an undemanded CAS entry: the dot prefix alone
	// is what must make gc blind to it, no matter how sweepable its insides look.
	let wiptree = td.path().join(".gar/treecas/.wiptree-planted");
	// Two levels under the dot dir, so the hash-named dir sits at the sweep's shard depth.
	let shardlike = wiptree.join(format!("bb/{}", "a".repeat(64)));
	std::fs::create_dir_all(&shardlike).expect("can plant a wiptree");
	std::fs::write(shardlike.join("f.txt"), "planted\n").expect("can plant a file");
	let demands = write_demands(td.path(), "keep.txt", &[]);

	let r = gar(td.path(), &["gc", "--demands", &demands]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(
		shardlike.join("f.txt").is_file(),
		"the planted wiptree must be untouched"
	);
}

/// `gc` needs a repo, same as `add`.
/// Pinned with an explicit `--repo` so no ancestor of the tempdir can satisfy a search.
#[test]
fn gc_without_a_repo_reports_no_repo() {
	let td = scratch("gc-no-repo");
	let demands = write_demands(td.path(), "keep.txt", &[]);
	let r = gar(
		td.path(),
		&["--repo", arg(td.path()), "gc", "--demands", &demands],
	);
	assert_eq!(r.code, 3, "stdout: {}, stderr: {}", r.stdout, r.stderr);
}
