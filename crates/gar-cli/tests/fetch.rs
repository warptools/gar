//! End-to-end tests for `gar fetch`, with the real binary, from heaps on disk and over HTTP.
//!
//! The exit codes and the stdout/stderr split are public interface, and pinned here:
//! stdout carries the fetched tree's path and nothing else.

mod common;
#[path = "../../gar/src/testsupport/static_server.rs"]
mod static_server;

use common::{arg, gar, scratch, treecas_entry, treeidx_entry};
use static_server::StaticServer;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;

/// Makes heaps at `root/remote` (holding a small tree, returned by hash) and `root/local` (empty).
fn remote_and_local(root: &Path) -> String {
	for name in ["remote", "local"] {
		fs::create_dir(root.join(name)).expect("mkdir");
		let r = gar(&root.join(name), &["init"]);
		assert_eq!(r.code, 0, "init {name}: {}", r.stderr);
	}
	let tree = root.join("remote/tree");
	fs::create_dir_all(tree.join("sub/empty")).expect("mkdir");
	fs::write(tree.join("file"), b"content\n").expect("write");
	fs::write(tree.join("sub/run.sh"), b"#!/bin/sh\n").expect("write");
	fs::set_permissions(tree.join("sub/run.sh"), fs::Permissions::from_mode(0o755)).expect("chmod");
	std::os::unix::fs::symlink("../file", tree.join("sub/link")).expect("symlink");
	let r = gar(&root.join("remote"), &["add", "tree"]);
	assert_eq!(r.code, 0, "add: {}", r.stderr);
	r.stdout.trim().to_owned()
}

/// The whole trip over HTTP: stdout is exactly the tree's path, and the heap it landed in is sound.
#[test]
fn fetch_over_http_end_to_end() {
	let td = scratch("cli-fetch-http");
	let hash = remote_and_local(td.path());
	let server = StaticServer::serve(&td.path().join("remote/.gar")).expect("can start a server");
	let local = td.path().join("local");

	let r = gar(&local, &["fetch", &hash, "--from", &server.url()]);

	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	let tree_path = treecas_entry(&local, &hash);
	assert_eq!(r.stdout, format!("{}\n", tree_path.display()));
	assert_eq!(
		r.stderr,
		"fetch: fetched 2 blobs (18 bytes); 0 blobs already in stock\n"
	);
	assert_eq!(gar(&local, &["path", &hash]).stdout, r.stdout);
	assert_eq!(gar(&local, &["hash", arg(&tree_path)]).stdout.trim(), hash);
	assert!(treeidx_entry(&local, &hash).is_file());
	let repair = gar(&local, &["repair"]);
	assert_eq!(repair.code, 0, "repair: {}", repair.stderr);

	// Again: nothing to do, and the same stdout.
	let again = gar(&local, &["fetch", &hash, "--from", &server.url()]);
	assert_eq!(again.code, 0, "stderr: {}", again.stderr);
	assert_eq!(again.stdout, r.stdout);
	assert!(
		again.stderr.contains("already in this heap"),
		"{}",
		again.stderr
	);
}

#[test]
fn fetch_of_a_tree_not_served_exits_6() {
	let td = scratch("cli-fetch-404");
	remote_and_local(td.path());
	let server = StaticServer::serve(&td.path().join("remote/.gar")).expect("can start a server");
	let absent = "ab".repeat(32);

	let r = gar(
		&td.path().join("local"),
		&["fetch", &absent, "--from", &server.url()],
	);

	assert_eq!(r.code, 6, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert!(
		r.stderr.starts_with("fetch: that tree isn't served there"),
		"{}",
		r.stderr
	);
	assert!(r.stderr.contains(&server.url()), "{}", r.stderr);
}

/// An index for some other tree, under the asked-for name: refused, nothing committed.
#[test]
fn fetch_refusing_a_lying_remote_exits_8() {
	let td = scratch("cli-fetch-lie");
	let hash = remote_and_local(td.path());
	let remote = td.path().join("remote");
	fs::create_dir(remote.join("other")).expect("mkdir");
	fs::write(remote.join("other/f"), b"other\n").expect("write");
	let other = gar(&remote, &["add", "other"]).stdout.trim().to_owned();
	// The stored index is read-only: replace it, don't overwrite.
	fs::remove_file(treeidx_entry(&remote, &hash)).expect("remove the stored index");
	fs::copy(
		treeidx_entry(&remote, &other),
		treeidx_entry(&remote, &hash),
	)
	.expect("swap index");
	let local = td.path().join("local");

	let r = gar(
		&local,
		&["fetch", &hash, "--from", arg(&remote.join(".gar"))],
	);

	assert_eq!(r.code, 8, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert!(
		r.stderr.contains(&other),
		"names the tree the index describes: {}",
		r.stderr
	);
	assert!(!treecas_entry(&local, &hash).exists());
}

/// A verified index naming a blob the remote has lost: the remote doesn't hold the tree.
#[test]
fn fetch_with_a_blob_missing_remotely_exits_6() {
	let td = scratch("cli-fetch-blob-missing");
	let hash = remote_and_local(td.path());
	let remote = td.path().join("remote");
	let blob = gar(&remote, &["hash", "tree/file"])
		.stdout
		.trim()
		.to_owned();
	fs::remove_file(common::blobcas_entry(&remote, &blob)).expect("remove a blob");

	let r = gar(
		&td.path().join("local"),
		&["fetch", &hash, "--from", arg(&remote.join(".gar"))],
	);

	assert_eq!(r.code, 6, "stderr: {}", r.stderr);
	assert!(r.stdout.is_empty(), "stdout: {}", r.stdout);
	assert!(
		r.stderr.contains("advertises blobs it doesn't hold") && r.stderr.contains(&blob),
		"{}",
		r.stderr
	);
}

/// Offers: stderr says which were used; a used one is the blobcas inode, an unused one is left alone.
#[test]
fn fetch_with_offers_reports_each() {
	let td = scratch("cli-fetch-offer");
	let hash = remote_and_local(td.path());
	let local = td.path().join("local");
	let offers = td.path().join("offers");
	fs::create_dir(&offers).expect("mkdir");
	fs::write(offers.join("file"), b"content\n").expect("write");
	fs::write(offers.join("stray"), b"nobody's\n").expect("write");
	let blob = gar(&td.path().join("remote"), &["hash", "tree/file"])
		.stdout
		.trim()
		.to_owned();

	let r = gar(
		&local,
		&[
			"fetch",
			"--from",
			arg(&td.path().join("remote/.gar")),
			"--offer",
			arg(&offers.join("file")),
			"--offer",
			arg(&offers.join("stray")),
			&hash,
		],
	);

	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert_eq!(
		r.stderr,
		format!(
			"fetch: fetched 1 blobs (10 bytes); 0 blobs already in stock\nfetch: offer {}: used\nfetch: offer {}: not used\n",
			offers.join("file").display(),
			offers.join("stray").display()
		)
	);
	assert_eq!(
		fs::metadata(offers.join("file")).expect("stat").ino(),
		fs::metadata(common::blobcas_entry(&local, &blob))
			.expect("stat")
			.ino()
	);
	assert_eq!(fs::metadata(offers.join("stray")).expect("stat").nlink(), 1);
}
