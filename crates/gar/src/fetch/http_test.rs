use std::ffi::OsStr;
use std::fs;
use std::io::Read as _;
use std::path::Path;

use super::{OpenError, Remote, Transport};
use crate::layout;
use crate::testsupport::static_server::StaticServer;

fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// Transports verify nothing, so any hashes will do as names.
fn hash(n: u8) -> gittree::Hash {
	gittree::Hash::from_hex(&format!("{n:02x}").repeat(32)).expect("valid hex")
}

/// Where the heap sits under the served root: below a prefix, so base URLs carry a path.
const HEAP_DIR: &str = "srv/heap";

fn place(root: &Path, rel: &layout::RelPath, body: &[u8]) {
	let path = root.join(HEAP_DIR).join(rel);
	fs::create_dir_all(path.parent().expect("relpaths have parents")).expect("can make shard dirs");
	fs::write(path, body).expect("can write a fixture file");
}

/// Picks the transport the way a caller would: by classifying the base URL.
fn transport_for(server: &StaticServer) -> Box<dyn Transport> {
	let base = format!("{}/{HEAP_DIR}", server.url());
	let remote = Remote::parse(OsStr::new(&base)).expect("a URL base");
	assert!(
		matches!(remote, Remote::Http(_)),
		"{base} should classify as HTTP"
	);
	remote.into_transport()
}

fn read_all(t: &dyn Transport, rel: &layout::RelPath) -> Vec<u8> {
	let mut got = Vec::new();
	t.open(rel)
		.unwrap_or_else(|e| panic!("opening {rel}: {e}"))
		.read_to_end(&mut got)
		.unwrap_or_else(|e| panic!("reading {rel}: {e}"));
	got
}

#[test]
fn reads_bytes_back() {
	let td = scratch("http-read");
	place(td.path(), &layout::blob(&hash(1), false), b"plain\n");
	place(td.path(), &layout::blob(&hash(1), true), b"#!/bin/sh\n");
	place(td.path(), &layout::index(&hash(2)), b"treeidx v1\n");
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let t = transport_for(&server);
	assert_eq!(read_all(&*t, &layout::blob(&hash(1), false)), b"plain\n");
	assert_eq!(read_all(&*t, &layout::blob(&hash(1), true)), b"#!/bin/sh\n");
	assert_eq!(read_all(&*t, &layout::index(&hash(2))), b"treeidx v1\n");
}

#[test]
fn a_404_is_not_found() {
	let td = scratch("http-404");
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let t = transport_for(&server);
	let rel = layout::index(&hash(3));
	match t.open(&rel) {
		Err(OpenError::NotFound {
			location,
		}) => {
			assert_eq!(location, format!("{}/{HEAP_DIR}/{rel}", server.url()));
		},
		Err(e) => panic!("expected NotFound, got {e}"),
		Ok(_) => panic!("expected NotFound, got a reader"),
	}
}

/// The test server answers a directory with a 500, standing in for every non-404 refusal.
#[test]
fn other_statuses_are_failures_naming_the_status() {
	let td = scratch("http-500");
	let rel = layout::index(&hash(4));
	fs::create_dir_all(td.path().join(HEAP_DIR).join(&rel))
		.expect("can make a dir where the file belongs");
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let t = transport_for(&server);
	match t.open(&rel) {
		Err(
			e @ OpenError::Failed {
				..
			},
		) => {
			let msg = e.to_string();
			assert!(
				msg.contains("500"),
				"the message names the status, got: {msg}"
			);
			assert!(
				msg.contains(rel.as_str()),
				"the message names the path, got: {msg}"
			);
		},
		Err(e) => panic!("expected Failed, got {e}"),
		Ok(_) => panic!("expected Failed, got a reader"),
	}
}

/// Nothing listening: the connection is refused, which is a failure, never absence.
#[test]
fn a_transport_error_is_a_failure() {
	let port = {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("can bind");
		listener.local_addr().expect("has an address").port()
	};
	let base = format!("http://127.0.0.1:{port}/heap");
	let t = Remote::parse(OsStr::new(&base))
		.expect("a URL base")
		.into_transport();
	match t.open(&layout::index(&hash(5))) {
		Err(OpenError::Failed {
			..
		}) => {},
		Err(e) => panic!("expected Failed, got {e}"),
		Ok(_) => panic!("expected Failed, got a reader"),
	}
}

/// Far larger than any buffer along the way, so the body has to stream through in pieces.
#[test]
fn a_large_body_streams_through_intact() {
	let td = scratch("http-large");
	let mut body = vec![0u8; 32 << 20];
	let mut x: u64 = 0x9e3779b97f4a7c15;
	for chunk in body.chunks_mut(8) {
		// xorshift64: cheap, and no repeating pattern a misplaced chunk could hide in.
		x ^= x << 13;
		x ^= x >> 7;
		x ^= x << 17;
		chunk.copy_from_slice(&x.to_le_bytes()[..chunk.len()]);
	}
	let rel = layout::blob(&hash(6), false);
	place(td.path(), &rel, &body);
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let t = transport_for(&server);
	let got = read_all(&*t, &rel);
	assert_eq!(got.len(), body.len());
	assert!(got == body, "the body came back changed");
}

/// Several requests through one transport: the pooled-connection path, whether or not ureq reuses.
#[test]
fn sequential_requests_on_one_transport() {
	let td = scratch("http-sequential");
	for n in 0..3 {
		place(
			td.path(),
			&layout::blob(&hash(n), false),
			format!("blob {n}\n").as_bytes(),
		);
	}
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let t = transport_for(&server);
	for n in 0..3 {
		let rel = layout::blob(&hash(n), false);
		assert_eq!(read_all(&*t, &rel), format!("blob {n}\n").as_bytes());
	}
	assert!(matches!(
		t.open(&layout::blob(&hash(9), false)),
		Err(OpenError::NotFound { .. })
	));
	assert_eq!(read_all(&*t, &layout::blob(&hash(0), false)), b"blob 0\n");
}

#[test]
fn a_trailing_slash_on_the_base_is_absorbed() {
	let rel = layout::index(&hash(7));
	let with = Remote::Http("http://example.com/heap/".into()).into_transport();
	let without = Remote::Http("http://example.com/heap".into()).into_transport();
	let want = format!("http://example.com/heap/{rel}");
	assert_eq!(with.location(&rel), want);
	assert_eq!(without.location(&rel), want);
}
