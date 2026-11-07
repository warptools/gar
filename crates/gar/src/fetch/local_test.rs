use std::fs;
use std::io::Read as _;

use super::{LocalTransport, OpenError, Transport};
use crate::layout;

fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// Transports verify nothing, so any hash will do as a name.
fn some_hash() -> gittree::Hash {
	gittree::Hash::from_hex("2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce")
		.expect("valid hex")
}

/// Puts `body` at `rel` under `base`, making the shard dirs it needs.
fn place(base: &std::path::Path, rel: &layout::RelPath, body: &[u8]) {
	let path = base.join(rel);
	fs::create_dir_all(path.parent().expect("relpaths have parents")).expect("can make shard dirs");
	fs::write(path, body).expect("can write a fixture file");
}

#[test]
fn reads_bytes_back() {
	let td = scratch("local-read");
	let h = some_hash();
	place(td.path(), &layout::blob(&h, false), b"plain\n");
	place(td.path(), &layout::blob(&h, true), b"#!/bin/sh\n");
	let t = LocalTransport::new(td.path());
	for (rel, want) in [
		(layout::blob(&h, false), &b"plain\n"[..]),
		(layout::blob(&h, true), &b"#!/bin/sh\n"[..]),
	] {
		let mut got = Vec::new();
		t.open(&rel)
			.expect("opening a present file")
			.read_to_end(&mut got)
			.expect("reading it");
		assert_eq!(got, want, "contents of {rel}");
	}
}

/// Both an absent entry in an existing shard dir and an absent shard dir are plain absence.
#[test]
fn absent_is_not_found() {
	let td = scratch("local-absent");
	let h = some_hash();
	place(td.path(), &layout::blob(&h, true), b"only the -x variant\n");
	let t = LocalTransport::new(td.path());
	for rel in [layout::blob(&h, false), layout::index(&h)] {
		match t.open(&rel) {
			Err(OpenError::NotFound {
				location,
			}) => {
				assert_eq!(location, td.path().join(&rel).display().to_string());
			},
			Err(e) => panic!("{rel}: expected NotFound, got {e}"),
			Ok(_) => panic!("{rel}: expected NotFound, got a reader"),
		}
	}
}

/// A directory opens fine on linux and only fails at the first read; the transport refuses it up front.
#[test]
fn a_directory_is_a_failure_not_a_file() {
	let td = scratch("local-dir");
	let rel = layout::index(&some_hash());
	fs::create_dir_all(td.path().join(&rel)).expect("can make a dir where the file belongs");
	let t = LocalTransport::new(td.path());
	match t.open(&rel) {
		Err(
			e @ OpenError::Failed {
				..
			},
		) => {
			let msg = e.to_string();
			assert!(msg.contains("not a regular file"), "got: {msg}");
			assert!(
				msg.contains(rel.as_str()),
				"the message names the path, got: {msg}"
			);
		},
		Err(e) => panic!("expected Failed, got {e}"),
		Ok(_) => panic!("expected Failed, got a reader"),
	}
}
