//! End-to-end tests for `gar repair`.
//!
//! Each test vandalizes a healthy heap in one specific way, proves the damage is
//! observable (a repair test that can't see its breakage proves nothing), then runs
//! repair and asserts the heap came back whole -- or that the corrupt entry landed
//! in quarantine with a `CORRUPTION_FOUND` exit, never deleted.

mod common;

use common::{arg, blobcas_entry, gar, scratch, treecas_entry};
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

/// Lays down the standard fixture tree and adds it: a plain file, an executable,
/// a symlink, a nested directory, and an empty directory (every entry kind repair walks).
/// Returns the treehash.
fn add_fixture_tree(root: &Path) -> String {
	let dir = root.join("stuff");
	fs::create_dir(&dir).expect("can make the tree dir");
	fs::write(dir.join("plain.txt"), "plain\n").expect("can write a file");
	fs::write(dir.join("run.sh"), "#!/bin/sh\necho hi\n").expect("can write a file");
	fs::set_permissions(dir.join("run.sh"), fs::Permissions::from_mode(0o755))
		.expect("can chmod the executable");
	std::os::unix::fs::symlink("plain.txt", dir.join("link")).expect("can make a symlink");
	fs::create_dir(dir.join("nested")).expect("can make a nested dir");
	fs::write(dir.join("nested/deep.txt"), "deep\n").expect("can write a file");
	fs::create_dir(dir.join("empty")).expect("can make an empty dir");
	let r = gar(root, &["add", "stuff"]);
	assert_eq!(r.code, 0, "add: stderr: {}", r.stderr);
	r.stdout.trim().to_owned()
}

/// The blob hash of a loose file, via `gar hash`.
fn blob_hash_of(root: &Path, path: &str) -> String {
	let r = gar(root, &["hash", path]);
	assert_eq!(r.code, 0, "hash {path}: stderr: {}", r.stderr);
	r.stdout.trim().to_owned()
}

fn ino(p: &Path) -> u64 {
	fs::metadata(p).expect("can stat").ino()
}

fn nlink(p: &Path) -> u64 {
	fs::metadata(p).expect("can stat").nlink()
}

fn mode(p: &Path) -> u32 {
	fs::metadata(p).expect("can stat").permissions().mode() & 0o7777
}

/// Recursively copies a directory the way the bozo tools do: file contents and modes
/// survive, hardlinks don't -- every file comes out on its own inode.
fn clone_breaking_links(src: &Path, dst: &Path) {
	fs::create_dir(dst).expect("can make the clone dir");
	for ent in fs::read_dir(src).expect("can read the source dir") {
		let ent = ent.expect("readable dirent");
		let ft = ent.file_type().expect("has a file type");
		let to = dst.join(ent.file_name());
		if ft.is_dir() {
			clone_breaking_links(&ent.path(), &to);
		} else if ft.is_symlink() {
			let target = fs::read_link(ent.path()).expect("can read the symlink");
			std::os::unix::fs::symlink(target, to).expect("can recreate the symlink");
		} else {
			fs::copy(ent.path(), to).expect("can copy a file");
		}
	}
}

/// Overwrites a read-only CAS file's bytes in place (same inode), as disk rot or a bad
/// sync tool would.  The mode round-trips so the damage is purely in the content.
fn corrupt_bytes(p: &Path, bytes: &[u8]) {
	let old_mode = mode(p);
	fs::set_permissions(p, fs::Permissions::from_mode(0o644)).expect("can unlock the file");
	fs::write(p, bytes).expect("can overwrite the file");
	fs::set_permissions(p, fs::Permissions::from_mode(old_mode)).expect("can relock the file");
}

/// The quarantine run dirs present in a heap (usually asserted to be zero or one).
fn quarantine_runs(root: &Path) -> Vec<PathBuf> {
	let q = root.join(".gar/quarantine");
	if !q.is_dir() {
		return Vec::new();
	}
	fs::read_dir(q)
		.expect("can read the quarantine dir")
		.map(|e| e.expect("readable dirent").path())
		.collect()
}

/// A heap copied with a link-breaking tool comes back: every treecas file ends up
/// sharing its blob's inode again, the tree still hash-verifies, and a second repair
/// finds nothing left to do.
#[test]
fn relink_after_linkbreaking_copy() {
	let td = scratch("repair-relink");
	let origin = td.path().join("origin");
	fs::create_dir(&origin).expect("can make the origin dir");
	gar(&origin, &["init"]);
	let tree = add_fixture_tree(&origin);

	let copy = td.path().join("copy");
	fs::create_dir(&copy).expect("can make the copy root");
	clone_breaking_links(&origin.join(".gar"), &copy.join(".gar"));

	// The vandalism is real: the copied treecas file sits alone on its inode.
	let tree_file = treecas_entry(&copy, &tree).join("plain.txt");
	assert_eq!(nlink(&tree_file), 1, "the copy must have broken the links");

	let r = gar(&copy, &["repair"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(
		r.stdout.is_empty(),
		"repair chatter belongs on stderr; stdout: {}",
		r.stdout
	);
	let blob = blobcas_entry(&copy, &blob_hash_of(&origin, "stuff/plain.txt"));
	assert_eq!(
		ino(&tree_file),
		ino(&blob),
		"the file links to its blob again"
	);
	let exec_blob_name = format!("{}-x", blob_hash_of(&origin, "stuff/run.sh"));
	let exec_blob = blobcas_entry(&copy, &exec_blob_name);
	assert_eq!(
		ino(&treecas_entry(&copy, &tree).join("run.sh")),
		ino(&exec_blob),
		"the executable links to its -x blob again"
	);
	// Whole means hash-verifiable, not merely relinked.
	let rehash = gar(&copy, &["hash", arg(&treecas_entry(&copy, &tree))]);
	assert_eq!(rehash.stdout.trim(), tree);
	assert!(quarantine_runs(&copy).is_empty(), "nothing was corrupt");

	// Idempotence: a second pass has nothing to repair.
	let r2 = gar(&copy, &["repair"]);
	assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
	assert!(
		r2.stderr
			.contains("relinked 0 files, donated 0 blobs, normalized 0 blob modes"),
		"the second pass must find nothing to do; stderr: {}",
		r2.stderr
	);
}

/// A wholly missing blobcas is rebuilt from treecas content alone.
#[test]
fn rebuild_missing_blobcas() {
	let td = scratch("repair-rebuild-blobcas");
	gar(td.path(), &["init"]);
	let tree = add_fixture_tree(td.path());
	let plain_blob = blobcas_entry(td.path(), &blob_hash_of(td.path(), "stuff/plain.txt"));
	let exec_blob = blobcas_entry(
		td.path(),
		&format!("{}-x", blob_hash_of(td.path(), "stuff/run.sh")),
	);

	fs::remove_dir_all(td.path().join(".gar/blobcas")).expect("can vandalize the blobcas");
	assert!(!plain_blob.exists(), "the blobcas must really be gone");

	let r = gar(td.path(), &["repair"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert_eq!(
		mode(&plain_blob),
		0o444,
		"the donated blob carries blob-normal mode"
	);
	assert_eq!(
		mode(&exec_blob),
		0o555,
		"the donated -x blob carries the exec mode"
	);
	assert_eq!(
		ino(&treecas_entry(td.path(), &tree).join("plain.txt")),
		ino(&plain_blob),
		"donation leaves the tree file and the blob as one inode"
	);
	let rehash = gar(td.path(), &["hash", arg(&treecas_entry(td.path(), &tree))]);
	assert_eq!(rehash.stdout.trim(), tree);
	assert!(quarantine_runs(td.path()).is_empty(), "nothing was corrupt");
}

/// Corrupt blob bytes: the blob is quarantined, and the tree whose file shared that
/// inode no longer recomputes to its name, so it's quarantined too.  Nothing is deleted.
#[test]
fn corrupt_blob_quarantines_blob_and_tree() {
	let td = scratch("repair-corrupt-blob");
	gar(td.path(), &["init"]);
	let tree = add_fixture_tree(td.path());
	let blob_hash = blob_hash_of(td.path(), "stuff/plain.txt");
	let blob = blobcas_entry(td.path(), &blob_hash);
	corrupt_bytes(&blob, b"evil\n");

	let r = gar(td.path(), &["repair"]);
	assert_eq!(
		r.code, 10,
		"corruption is a distinct exit; stderr: {}",
		r.stderr
	);
	assert!(!blob.exists(), "the corrupt blob is out of circulation");
	assert!(
		!treecas_entry(td.path(), &tree).exists(),
		"the poisoned tree is out of circulation"
	);
	let runs = quarantine_runs(td.path());
	assert_eq!(runs.len(), 1, "exactly one quarantine run dir");
	assert!(
		runs[0]
			.join("blobcas")
			.join(&blob_hash[0..2])
			.join(&blob_hash[2..4])
			.join(&blob_hash)
			.is_file(),
		"the corrupt blob sits in quarantine under its sharded path"
	);
	assert!(
		runs[0]
			.join("treecas")
			.join(&tree[0..2])
			.join(&tree[2..4])
			.join(&tree)
			.join("plain.txt")
			.is_file(),
		"the quarantined tree is intact for forensics"
	);
	assert!(
		r.stderr.contains(arg(&runs[0])),
		"the quarantine path is reported; stderr: {}",
		r.stderr
	);
}

/// A stripped exec bit (on a broken-link copy of a file) changes what the tree hashes to,
/// which is corruption repair cannot invent its way out of: the tree is quarantined.
#[test]
fn stripped_exec_bit_quarantines_tree() {
	let td = scratch("repair-stripped-exec");
	gar(td.path(), &["init"]);
	let tree = add_fixture_tree(td.path());
	let tree_file = treecas_entry(td.path(), &tree).join("run.sh");
	let bytes = fs::read(&tree_file).expect("can read the file");
	fs::remove_file(&tree_file).expect("can remove the tree file");
	fs::write(&tree_file, bytes).expect("can rewrite it");
	fs::set_permissions(&tree_file, fs::Permissions::from_mode(0o644))
		.expect("can strip the exec bit");

	let r = gar(td.path(), &["repair"]);
	assert_eq!(r.code, 10, "stderr: {}", r.stderr);
	assert!(
		!treecas_entry(td.path(), &tree).exists(),
		"the tree no longer recomputes to its name and must leave circulation"
	);
	// The untouched -x blob was and stays verified; only the tree was condemned.
	let exec_blob = blobcas_entry(
		td.path(),
		&format!("{}-x", blob_hash_of(td.path(), "stuff/run.sh")),
	);
	assert!(exec_blob.is_file(), "the healthy blob stays in the blobcas");
}

/// Drifted blob modes are normalized to what the name claims: the `-x` suffix is
/// authoritative, in both directions.  No quarantine, exit 0.
#[test]
fn mode_drift_is_normalized() {
	let td = scratch("repair-mode-drift");
	gar(td.path(), &["init"]);
	let tree = add_fixture_tree(td.path());
	let plain_blob = blobcas_entry(td.path(), &blob_hash_of(td.path(), "stuff/plain.txt"));
	let exec_blob = blobcas_entry(
		td.path(),
		&format!("{}-x", blob_hash_of(td.path(), "stuff/run.sh")),
	);
	fs::set_permissions(&plain_blob, fs::Permissions::from_mode(0o644))
		.expect("can drift the plain blob's mode");
	fs::set_permissions(&exec_blob, fs::Permissions::from_mode(0o444))
		.expect("can strip the -x blob's exec bit");

	let r = gar(td.path(), &["repair"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert_eq!(mode(&plain_blob), 0o444);
	assert_eq!(
		mode(&exec_blob),
		0o555,
		"the -x name wins over the drifted mode"
	);
	assert!(
		quarantine_runs(td.path()).is_empty(),
		"mode drift is not corruption"
	);
	// The shared inode means the treecas file got its exec bit back too; the tree verifies.
	let rehash = gar(td.path(), &["hash", arg(&treecas_entry(td.path(), &tree))]);
	assert_eq!(rehash.stdout.trim(), tree);
}

/// A healthy heap is a no-op: everything verifies, nothing changes, exit 0.
#[test]
fn healthy_heap_is_a_noop() {
	let td = scratch("repair-healthy");
	gar(td.path(), &["init"]);
	let tree = add_fixture_tree(td.path());
	let tree_file = treecas_entry(td.path(), &tree).join("plain.txt");
	let ino_before = ino(&tree_file);

	let r = gar(td.path(), &["repair"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(
		r.stderr
			.contains("relinked 0 files, donated 0 blobs, normalized 0 blob modes"),
		"nothing to repair; stderr: {}",
		r.stderr
	);
	assert_eq!(ino(&tree_file), ino_before, "no inode was disturbed");
	assert!(quarantine_runs(td.path()).is_empty());
	assert!(
		!td.path().join(".gar/quarantine").exists(),
		"a clean repair leaves no quarantine dir at all"
	);
}
