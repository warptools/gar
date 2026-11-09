//! Tests for `fetch::fetch_tree` succeeding, between heaps on disk, over the local-filesystem transport.
//!
//! The remote is a heap made by `add`; the cross-checks are `gittree::hash_of_path`
//! (the fetched tree must recompute to the hash asked for) and the remote's own index bytes.

mod fetch_support;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::Path;

use fetch_support::{
	BIG_SIZE, blob_rel, heap, leftover_temps, recording_from, remote_index, remote_with_specimen,
	scratch,
};
use gar::add;
use gar::fetch::{self, Outcome};
use gar::layout;
use gar::treeidx;

#[test]
fn full_fetch_into_an_empty_heap() {
	let td = scratch("fetch-full");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let rec = recording_from(&remote);

	let outcome = fetch::fetch_tree(&local, &rec, &hash, &[]).expect("fetch succeeds");

	let remote_idx = remote_index(&remote, &hash);
	let file_bytes: u64 = [7, 7, 7, 9, 18, BIG_SIZE as u64].iter().sum();
	assert_eq!(
		outcome,
		Outcome::Fetched {
			blobs_fetched: 6,
			bytes_fetched: file_bytes,
			blobs_in_stock: 0,
			offers_used: vec![],
		}
	);

	// The index first, then each distinct blob exactly once.
	let opens = rec.opens();
	assert_eq!(opens[0], layout::index(&hash).as_str());
	let blob_opens: HashSet<&String> = opens[1..].iter().collect();
	assert_eq!(opens.len(), 7, "opens: {opens:#?}");
	assert_eq!(blob_opens.len(), 6, "a blob was fetched twice: {opens:#?}");

	let tree = local.tree_path(&hash);
	assert_eq!(gittree::hash_of_path(&tree).expect("hashable"), hash);
	assert_eq!(
		fs::read(local.index_path(&hash)).expect("local index"),
		fs::read(remote.index_path(&hash)).expect("remote index"),
	);

	// Every file is a hardlink into the blobcas, and every blob has its read-only mode.
	for entry in remote_idx.entries() {
		let treeidx::Kind::File {
			hash: blob,
			executable,
			..
		} = &entry.kind
		else {
			continue;
		};
		let in_tree = fs::metadata(tree.join(OsStr::from_bytes(&entry.path))).expect("stat");
		let in_blobcas = fs::metadata(local.blob_path(blob, *executable)).expect("stat");
		assert_eq!(
			in_tree.ino(),
			in_blobcas.ino(),
			"{} is not linked to its blob",
			entry.path.escape_ascii()
		);
		let want_mode = if *executable { 0o555 } else { 0o444 };
		assert_eq!(in_blobcas.permissions().mode() & 0o777, want_mode);
	}

	// The hash covers these, but a direct look makes a failure say what went wrong.
	assert_eq!(
		fs::read_link(tree.join("spaced_target")).expect("a symlink"),
		Path::new("a target with spaces")
	);
	assert_eq!(
		fs::read_dir(tree.join("nested/deeper/hollow"))
			.expect("an empty dir")
			.count(),
		0
	);
	assert!(tree.join("empty").is_dir());

	assert_eq!(leftover_temps(&local), Vec::<std::path::PathBuf>::new());
}

/// Blobs already in the local heap are not fetched; only the gap is.
#[test]
fn partial_stock_fetches_only_the_gap() {
	let td = scratch("fetch-partial");
	let (remote, hash) = remote_with_specimen(td.path());
	let remote_idx = remote_index(&remote, &hash);
	let local = heap(td.path(), "local");
	// Stock two of the specimen's blobs locally: "a file\n" (plain), and the big one.
	let partial = td.path().join("partial");
	fs::create_dir(&partial).expect("mkdir");
	fs::write(partial.join("x"), b"a file\n").expect("write");
	fs::copy(td.path().join("specimen/nested/big"), partial.join("y")).expect("copy");
	add::add(&local, &partial, add::FaithMode::Copy).expect("add the partial tree");
	let rec = recording_from(&remote);

	let outcome = fetch::fetch_tree(&local, &rec, &hash, &[]).expect("fetch succeeds");

	assert_eq!(
		outcome,
		Outcome::Fetched {
			blobs_fetched: 4,
			bytes_fetched: 7 + 7 + 9 + 18,
			blobs_in_stock: 2,
			offers_used: vec![],
		}
	);
	let opens: HashSet<String> = rec.opens().into_iter().collect();
	let want: HashSet<String> = [
		layout::index(&hash),
		blob_rel(&remote_idx, "exec_twin"),
		blob_rel(&remote_idx, "has space"),
		blob_rel(&remote_idx, "nested/new\nline"),
		blob_rel(&remote_idx, "nested/run.sh"),
	]
	.iter()
	.map(|r| r.as_str().to_owned())
	.collect();
	assert_eq!(opens, want);
	assert_eq!(rec.opens().len(), 5, "something was opened twice");
	assert_eq!(
		gittree::hash_of_path(local.tree_path(&hash)).expect("hashable"),
		hash
	);
}

/// A tree already in the heap is success with nothing read, however it got there.
#[test]
fn a_tree_already_here_reads_nothing() {
	let td = scratch("fetch-already");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let again = add::add(&local, td.path().join("specimen"), add::FaithMode::Copy)
		.expect("add the same tree locally");
	assert_eq!(again, hash);
	let rec = recording_from(&remote);

	let outcome = fetch::fetch_tree(&local, &rec, &hash, &[]).expect("fetch succeeds");

	assert_eq!(outcome, Outcome::AlreadyInStock);
	assert_eq!(rec.opens(), Vec::<String>::new());
}

/// Fetching twice: the second is a no-op.
#[test]
fn fetch_is_idempotent() {
	let td = scratch("fetch-twice");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	fetch::fetch_tree(&local, &recording_from(&remote), &hash, &[]).expect("first fetch");
	let rec = recording_from(&remote);
	assert_eq!(
		fetch::fetch_tree(&local, &rec, &hash, &[]).expect("second fetch"),
		Outcome::AlreadyInStock
	);
	assert_eq!(rec.opens(), Vec::<String>::new());
}

/// A fetched heap serves the tree exactly as its source did: fetching onward from it lands the same bytes.
#[test]
fn a_fetched_heap_serves_onward() {
	let td = scratch("fetch-chain");
	let (remote, hash) = remote_with_specimen(td.path());
	let middle = heap(td.path(), "middle");
	let last = heap(td.path(), "last");

	fetch::fetch_tree(&middle, &recording_from(&remote), &hash, &[]).expect("first hop");
	let rec = recording_from(&middle);
	let outcome = fetch::fetch_tree(&last, &rec, &hash, &[]).expect("second hop");

	assert!(matches!(
		outcome,
		Outcome::Fetched {
			blobs_fetched: 6,
			..
		}
	));
	let original = fs::read(remote.index_path(&hash)).expect("remote index");
	assert_eq!(
		fs::read(middle.index_path(&hash)).expect("middle index"),
		original
	);
	assert_eq!(
		fs::read(last.index_path(&hash)).expect("last index"),
		original
	);
	assert_eq!(
		gittree::hash_of_path(last.tree_path(&hash)).expect("hashable"),
		hash
	);
}
