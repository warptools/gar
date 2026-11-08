//! Tests for `fetch::fetch_tree` over HTTP, against a dumb static server pointed at a heap.

mod fetch_support;
#[path = "../src/testsupport/static_server.rs"]
mod static_server;

use std::ffi::OsStr;
use std::fs;

use fetch_support::{BIG_SIZE, heap, leftover_temps, remote_with_specimen, scratch};
use gar::fetch::{self, FetchError, Outcome};
use gar::layout;
use static_server::StaticServer;

/// The transport a caller gets for the URL of `server`'s heap, which sits at `{root}/remote/.gar`.
fn http_transport(server: &StaticServer) -> Box<dyn fetch::Transport> {
	let base = format!("{}/remote/.gar", server.url());
	let remote = fetch::Remote::parse(OsStr::new(&base)).expect("a URL");
	assert!(matches!(remote, fetch::Remote::Http(_)));
	remote.into_transport()
}

#[test]
fn full_fetch_over_http() {
	let td = scratch("fetch-http-full");
	let (remote, hash) = remote_with_specimen(td.path());
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let local = heap(td.path(), "local");

	let outcome = fetch::fetch_tree(&local, &*http_transport(&server), &hash).expect("fetch");

	assert_eq!(
		outcome,
		Outcome::Fetched {
			blobs_fetched: 6,
			bytes_fetched: 7 + 7 + 7 + 9 + 18 + BIG_SIZE as u64,
			blobs_in_stock: 0,
		}
	);
	assert_eq!(
		gittree::hash_of_path(local.tree_path(&hash)).expect("hashable"),
		hash
	);
	assert_eq!(
		fs::read(local.index_path(&hash)).expect("local index"),
		fs::read(remote.index_path(&hash)).expect("remote index"),
	);
	assert_eq!(leftover_temps(&local), Vec::<std::path::PathBuf>::new());
}

#[test]
fn an_index_404_is_not_served() {
	let td = scratch("fetch-http-404");
	let (remote, hash) = remote_with_specimen(td.path());
	fs::remove_file(remote.index_path(&hash)).expect("remove the index");
	let server = StaticServer::serve(td.path()).expect("can start a server");
	let local = heap(td.path(), "local");

	match fetch::fetch_tree(&local, &*http_transport(&server), &hash) {
		Err(FetchError::NotServed {
			location,
		}) => assert_eq!(
			location,
			format!("{}/remote/.gar/{}", server.url(), layout::index(&hash))
		),
		r => panic!("expected NotServed, got {r:?}"),
	}
	assert!(!local.tree_path(&hash).exists());
}
