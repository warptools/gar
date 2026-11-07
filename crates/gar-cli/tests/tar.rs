//! End-to-end tests for the tar transport: `gar export` and `gar import`.
//!
//! The properties pinned here are the transport's whole value:
//! export is deterministic, the round trip is lossless (hash-verifiably so),
//! and import fails closed against hostile streams.

mod common;

use common::{arg, gar, gar_bytes, scratch, treecas_entry};
use rstest::rstest;
use std::path::{Path, PathBuf};

/// Plants the transport specimen and adds it: a nested dir, an executable, a symlink,
/// and an empty dir -- every entry kind the transport carries,
/// including gar's one divergence from git (the empty dir).  Returns the treehash.
fn plant_and_add(root: &Path) -> String {
	let dir = root.join("specimen");
	std::fs::create_dir_all(dir.join("nested")).expect("can make the specimen dirs");
	std::fs::create_dir(dir.join("empty_dir")).expect("can make the empty dir");
	std::fs::write(dir.join("file.txt"), "hello\n").expect("can write a file");
	std::fs::write(dir.join("nested/run.sh"), "#!/bin/sh\necho hi\n").expect("can write a script");
	std::fs::set_permissions(
		dir.join("nested/run.sh"),
		std::os::unix::fs::PermissionsExt::from_mode(0o755),
	)
	.expect("can chmod the script");
	std::os::unix::fs::symlink("nested/run.sh", dir.join("link")).expect("can make a symlink");
	let r = gar(root, &["add", "specimen"]);
	assert_eq!(r.code, 0, "add specimen: stderr: {}", r.stderr);
	r.stdout.trim().to_owned()
}

/// Exports `hash` into a file named `name` under `root`, returning its path.
fn export_to_file(root: &Path, hash: &str, name: &str) -> PathBuf {
	let r = gar_bytes(root, &["export", hash], None);
	assert_eq!(r.code, 0, "export: stderr: {}", r.stderr);
	assert!(!r.stdout.is_empty(), "an export of a real tree has bytes");
	let path = root.join(name);
	std::fs::write(&path, &r.stdout).expect("can save the export");
	path
}

/// A fresh heap in its own scratch dir, so imports can't lean on blobs already in stock.
fn fresh_heap(name: &str) -> tempdir::TempDir {
	let td = scratch(name);
	let r = gar(td.path(), &["init"]);
	assert_eq!(r.code, 0, "init: stderr: {}", r.stderr);
	td
}

/// The full product promise in one flow: export here, import --expect there,
/// same hash out, and the received tree re-hashes to it bit for bit.
#[test]
fn round_trip_preserves_the_tree() {
	let src = fresh_heap("tar-roundtrip-src");
	let hash = plant_and_add(src.path());
	let tarfile = export_to_file(src.path(), &hash, "specimen.tar");

	let dst = fresh_heap("tar-roundtrip-dst");
	let r = gar_bytes(dst.path(), &["import", "--expect", &hash], Some(&tarfile));
	assert_eq!(r.code, 0, "import: stderr: {}", r.stderr);
	assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), hash);
	// Received means hash-verifiable in place, not merely present.
	let entry = treecas_entry(dst.path(), &hash);
	let rehash = gar(dst.path(), &["hash", arg(&entry)]);
	assert_eq!(rehash.stdout.trim(), hash, "stderr: {}", rehash.stderr);
}

/// The same tree exports as byte-identical tar, every time.
#[test]
fn export_is_deterministic() {
	let td = fresh_heap("tar-determinism");
	let hash = plant_and_add(td.path());
	let a = gar_bytes(td.path(), &["export", &hash], None);
	let b = gar_bytes(td.path(), &["export", &hash], None);
	assert_eq!(a.code, 0);
	assert_eq!(b.code, 0);
	assert!(!a.stdout.is_empty());
	assert_eq!(
		a.stdout, b.stdout,
		"two exports of one tree must not differ"
	);
}

/// Exporting a tree the heap doesn't hold is the same misunderstanding as `gar path` of it:
/// same exit code, and the diagnostic names the hash.
#[test]
fn export_of_an_absent_tree_is_not_in_stock() {
	let td = fresh_heap("tar-export-absent");
	let absent = "0000000000000000000000000000000000000000000000000000000000000000";
	let r = gar_bytes(td.path(), &["export", absent], None);
	assert_eq!(r.code, 6, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "no tar bytes for an absent tree");
	assert!(
		r.stderr.contains(absent),
		"the diagnostic should name the hash; stderr: {}",
		r.stderr
	);
}

/// A mismatched --expect is a failing exit naming both hashes -- but the stored tree
/// legitimately stays in the heap: a CAS is append-only, and removal is gc's job.
#[test]
fn import_expect_catches_a_mismatch() {
	let src = fresh_heap("tar-mismatch-src");
	let hash = plant_and_add(src.path());
	let tarfile = export_to_file(src.path(), &hash, "specimen.tar");
	let wanted = "1111111111111111111111111111111111111111111111111111111111111111";

	let dst = fresh_heap("tar-mismatch-dst");
	let r = gar_bytes(dst.path(), &["import", "--expect", wanted], Some(&tarfile));
	assert_eq!(r.code, 8, "stderr: {}", r.stderr);
	assert!(
		r.stderr.contains(&hash) && r.stderr.contains(wanted),
		"the diagnostic should name both hashes; stderr: {}",
		r.stderr
	);
	assert!(
		treecas_entry(dst.path(), &hash).is_dir(),
		"the stored tree stays in the heap under its real hash"
	);
}

/// A one-entry archive whose header is written raw: the entry name can hold
/// what `Header::set_path` would (rightly) refuse to encode.
fn tar_with_raw_name(name: &[u8], entry_type: tar::EntryType, content: &[u8]) -> Vec<u8> {
	let mut header = tar::Header::new_gnu();
	header.as_mut_bytes()[..name.len()].copy_from_slice(name);
	header.set_entry_type(entry_type);
	header.set_size(content.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	let mut buf = Vec::new();
	let mut b = tar::Builder::new(&mut buf);
	b.append(&header, content).expect("in-memory tar write");
	b.finish().expect("in-memory tar finish");
	drop(b);
	buf
}

/// Escaping paths are refused by name, and nothing lands outside the staging area.
#[rstest]
#[case::dotdot(b"../escapee".as_slice())]
#[case::absolute(b"/escapee".as_slice())]
fn import_refuses_escaping_paths(#[case] name: &[u8]) {
	let td = fresh_heap("tar-traversal");
	let evil = tar_with_raw_name(name, tar::EntryType::Regular, b"gotcha\n");
	let tarfile = td.path().join("evil.tar");
	std::fs::write(&tarfile, evil).expect("can save the evil tar");

	let r = gar_bytes(td.path(), &["import"], Some(&tarfile));
	assert_eq!(r.code, 9, "stdout: {:?}, stderr: {}", r.stdout, r.stderr);
	assert!(
		r.stderr.contains("escapee"),
		"the refusal should name the entry; stderr: {}",
		r.stderr
	);
	// Staging sits directly in `.gar/`, so a `../` escape from it would land here.
	assert!(
		!td.path().join(".gar/escapee").exists() && !td.path().join("escapee").exists(),
		"nothing may land outside the staging area"
	);
}

/// Entry types outside {file, dir, symlink} are refused by name.  Fifos have no CAS
/// representation at all; hardlink entries are links into *the archive's* namespace,
/// which a tree snapshot doesn't have.
#[rstest]
#[case::fifo(tar::EntryType::Fifo)]
#[case::hardlink(tar::EntryType::Link)]
fn import_refuses_unsupported_entry_types(#[case] entry_type: tar::EntryType) {
	let td = fresh_heap("tar-unsupported");
	let mut header = tar::Header::new_gnu();
	header.set_path("oddball").expect("a plain name encodes");
	if entry_type == tar::EntryType::Link {
		header
			.set_link_name("elsewhere")
			.expect("a plain target encodes");
	}
	header.set_entry_type(entry_type);
	header.set_size(0);
	header.set_mode(0o644);
	header.set_cksum();
	let mut buf = Vec::new();
	let mut b = tar::Builder::new(&mut buf);
	b.append(&header, &[][..]).expect("in-memory tar write");
	b.finish().expect("in-memory tar finish");
	drop(b);
	let tarfile = td.path().join("odd.tar");
	std::fs::write(&tarfile, buf).expect("can save the odd tar");

	let r = gar_bytes(td.path(), &["import"], Some(&tarfile));
	assert_eq!(r.code, 9, "stdout: {:?}, stderr: {}", r.stdout, r.stderr);
	assert!(
		r.stderr.contains("oddball"),
		"the refusal should name the entry; stderr: {}",
		r.stderr
	);
}

/// The two-step escape: a symlink pointing outside, then a file written through it.
/// Both entries pass gar's own path checks (all-normal components), so the refusal here
/// belongs to the tar crate's unpacker -- this test is us double-checking that lane.
/// The exact exit code is not promised (today it rides the io-error path);
/// what's pinned is that it fails, and that nothing lands outside.
#[test]
fn import_refuses_symlink_traversal() {
	let td = fresh_heap("tar-symlink-traversal");
	let outside = td.path().join("outside");
	std::fs::create_dir(&outside).expect("can make the outside dir");
	let mut buf = Vec::new();
	let mut b = tar::Builder::new(&mut buf);
	// Entry 1: a symlink pointing outside the staging area.
	let mut h = tar::Header::new_gnu();
	h.set_path("sneaky").expect("encodes");
	h.set_entry_type(tar::EntryType::Symlink);
	h.set_link_name(outside.to_str().expect("utf8"))
		.expect("encodes");
	h.set_size(0);
	h.set_mode(0o777);
	h.set_cksum();
	b.append(&h, &[][..]).expect("write");
	// Entry 2: a file whose path goes through the symlink.
	let mut h = tar::Header::new_gnu();
	h.set_path("sneaky/evil").expect("encodes");
	h.set_entry_type(tar::EntryType::Regular);
	h.set_size(7);
	h.set_mode(0o644);
	h.set_cksum();
	b.append(&h, &b"gotcha\n"[..]).expect("write");
	b.finish().expect("finish");
	drop(b);
	let tarfile = td.path().join("evil.tar");
	std::fs::write(&tarfile, buf).expect("can save the evil tar");

	let r = gar_bytes(td.path(), &["import"], Some(&tarfile));
	assert_ne!(r.code, 0, "a traversal attempt must not be a success");
	assert!(r.stdout.is_empty(), "no treehash for a refused stream");
	assert!(
		!outside.join("evil").exists(),
		"nothing may land outside the staging area"
	);
}

/// The same-name replacement game, directory variant: a symlink pointing outside,
/// then a directory entry at the same name, hoping the directory handling
/// (the unpacker's, or our own permission normalization) lands on the symlink's target.
/// Nothing outside may be created, modified, or re-moded.
#[test]
fn import_refuses_dir_over_symlink_replacement() {
	use std::os::unix::fs::PermissionsExt as _;
	let td = fresh_heap("tar-dir-over-symlink");
	let outside = td.path().join("outside");
	std::fs::create_dir(&outside).expect("can make the outside dir");
	std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700))
		.expect("can set a sentinel mode");
	let mut buf = Vec::new();
	let mut b = tar::Builder::new(&mut buf);
	let mut h = tar::Header::new_gnu();
	h.set_path("sneaky").expect("encodes");
	h.set_entry_type(tar::EntryType::Symlink);
	h.set_link_name(outside.to_str().expect("utf8"))
		.expect("encodes");
	h.set_size(0);
	h.set_mode(0o777);
	h.set_cksum();
	b.append(&h, &[][..]).expect("write");
	let mut h = tar::Header::new_gnu();
	h.set_path("sneaky/").expect("encodes");
	h.set_entry_type(tar::EntryType::Directory);
	h.set_size(0);
	h.set_mode(0o755);
	h.set_cksum();
	b.append(&h, &[][..]).expect("write");
	b.finish().expect("finish");
	drop(b);
	let tarfile = td.path().join("evil.tar");
	std::fs::write(&tarfile, buf).expect("can save the evil tar");

	let r = gar_bytes(td.path(), &["import"], Some(&tarfile));
	assert_ne!(
		r.code, 0,
		"a dir entry landing on a symlink must not be a success"
	);
	let mode = std::fs::metadata(&outside)
		.expect("stat outside")
		.permissions()
		.mode()
		& 0o7777;
	assert_eq!(mode, 0o700, "the outside dir's mode must be untouched");
	assert_eq!(
		std::fs::read_dir(&outside).expect("read outside").count(),
		0,
		"nothing may land inside the outside dir"
	);
}

/// The same-name replacement game, file variant: a symlink pointing at a file outside,
/// then a regular file entry at the same name, hoping the write goes through the link.
/// This stream is legal tar (duplicate names, last entry wins), so the import *succeeds* --
/// what's pinned is how: the symlink is replaced by the file, never written through,
/// so the outside file's content is untouched and the stored tree holds a plain file.
#[test]
fn import_replaces_a_symlink_rather_than_writing_through_it() {
	let td = fresh_heap("tar-file-over-symlink");
	let victim = td.path().join("victim.txt");
	std::fs::write(&victim, "precious\n").expect("can write the victim file");
	let mut buf = Vec::new();
	let mut b = tar::Builder::new(&mut buf);
	let mut h = tar::Header::new_gnu();
	h.set_path("sneaky").expect("encodes");
	h.set_entry_type(tar::EntryType::Symlink);
	h.set_link_name(victim.to_str().expect("utf8"))
		.expect("encodes");
	h.set_size(0);
	h.set_mode(0o777);
	h.set_cksum();
	b.append(&h, &[][..]).expect("write");
	let mut h = tar::Header::new_gnu();
	h.set_path("sneaky").expect("encodes");
	h.set_entry_type(tar::EntryType::Regular);
	h.set_size(7);
	h.set_mode(0o644);
	h.set_cksum();
	b.append(&h, &b"gotcha\n"[..]).expect("write");
	b.finish().expect("finish");
	drop(b);
	let tarfile = td.path().join("evil.tar");
	std::fs::write(&tarfile, buf).expect("can save the evil tar");

	let r = gar_bytes(td.path(), &["import"], Some(&tarfile));
	assert_eq!(
		r.code, 0,
		"a legal duplicate-name stream imports; stderr: {}",
		r.stderr
	);
	assert_eq!(
		std::fs::read_to_string(&victim).expect("read victim"),
		"precious\n",
		"the outside file's content must be untouched"
	);
	let stored = treecas_entry(td.path(), String::from_utf8_lossy(&r.stdout).trim()).join("sneaky");
	let meta = std::fs::symlink_metadata(&stored).expect("stat the stored entry");
	assert!(
		meta.is_file(),
		"the last entry won, as a plain file, not a symlink"
	);
	assert_eq!(
		std::fs::read_to_string(&stored).expect("read stored"),
		"gotcha\n"
	);
}
