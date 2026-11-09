//! Tests for `fetch::fetch_tree` with offers: local files that may be blobs the tree needs.
//!
//! An offer that matches a blob the heap lacks becomes the blobcas entry (same inode) and the remote's copy is never read;
//! every other offer is left alone, and an offer that can't be linked refuses the whole fetch.

mod fetch_support;

use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use fetch_support::{
	BIG_SIZE, assert_nothing_committed, blob_count, blob_of, blob_rel, heap, leftover_temps,
	recording_from, remote_index, remote_with_specimen, scratch,
};
use gar::add;
use gar::fetch::{self, Outcome};

/// Writes an offer file at `dir/name` with `body` and `mode`.
fn offer(dir: &Path, name: &str, body: &[u8], mode: u32) -> PathBuf {
	fs::create_dir_all(dir).expect("mkdir");
	let path = dir.join(name);
	fs::write(&path, body).expect("write");
	fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("chmod");
	path
}

/// Two offers matching needed blobs, each with the mode opposite to what the index calls for:
/// both are used, the remote's copies are never opened, and the blobcas entries are the offered inodes with the blobcas modes.
#[test]
fn offers_matching_needed_blobs_are_linked_in() {
	let td = scratch("fetch-offer-used");
	let (remote, hash) = remote_with_specimen(td.path());
	let remote_idx = remote_index(&remote, &hash);
	let local = heap(td.path(), "local");
	let offers_dir = td.path().join("offers");
	let big = offer(
		&offers_dir,
		"big",
		&fs::read(td.path().join("specimen/nested/big")).expect("read"),
		0o755,
	);
	let run = offer(&offers_dir, "run", b"#!/bin/sh\necho hi\n", 0o644);
	let rec = recording_from(&remote);

	let outcome = fetch::fetch_tree(&local, &rec, &hash, &[big.clone(), run.clone()])
		.expect("fetch succeeds");

	assert_eq!(
		outcome,
		Outcome::Fetched {
			blobs_fetched: 4,
			bytes_fetched: 7 + 7 + 7 + 9,
			blobs_in_stock: 0,
			offers_used: vec![big.clone(), run.clone()],
		}
	);
	let opens = rec.opens();
	for path in ["nested/big", "nested/run.sh"] {
		let rel = blob_rel(&remote_idx, path);
		assert!(
			!opens.contains(&rel.as_str().to_owned()),
			"{rel} was read from the remote"
		);
	}
	for (path, in_tree, want_mode) in [(&big, "nested/big", 0o444), (&run, "nested/run.sh", 0o555)]
	{
		let (blob, executable) = blob_of(&remote_idx, in_tree);
		let offered = fs::metadata(path).expect("the offer is still at its path");
		let stocked = fs::metadata(local.blob_path(&blob, executable)).expect("stocked");
		assert_eq!(
			offered.ino(),
			stocked.ino(),
			"{} is not the blobcas inode",
			path.display()
		);
		assert_eq!(stocked.permissions().mode() & 0o777, want_mode);
	}
	assert_eq!(fs::metadata(&big).expect("stat").len() as usize, BIG_SIZE);
	assert_eq!(
		gittree::hash_of_path(local.tree_path(&hash)).expect("hashable"),
		hash
	);
	assert_eq!(leftover_temps(&local), Vec::<PathBuf>::new());
}

/// An offer matching nothing in the tree: not used, nothing about it changes, and the fetch is a normal full fetch.
#[test]
fn an_offer_matching_nothing_is_left_alone() {
	let td = scratch("fetch-offer-unmatched");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let stray = offer(
		&td.path().join("offers"),
		"stray",
		b"nothing wants this\n",
		0o640,
	);
	let before = fs::metadata(&stray).expect("stat");

	let outcome = fetch::fetch_tree(&local, &recording_from(&remote), &hash, &[stray.clone()])
		.expect("fetch succeeds");

	assert!(
		matches!(
			outcome,
			Outcome::Fetched {
				blobs_fetched: 6,
				ref offers_used,
				..
			} if offers_used.is_empty()
		),
		"{outcome:?}"
	);
	let after = fs::metadata(&stray).expect("the offer is still at its path");
	assert_eq!(after.nlink(), 1, "the offer was linked somewhere");
	assert_eq!(after.permissions().mode(), before.permissions().mode());
	assert_eq!(fs::read(&stray).expect("read"), b"nothing wants this\n");
}

/// An offer matching a blob the heap already holds: the stocked blob is used, and the offer is left alone.
#[test]
fn an_offer_for_a_blob_in_stock_is_left_alone() {
	let td = scratch("fetch-offer-in-stock");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let big_body = fs::read(td.path().join("specimen/nested/big")).expect("read");
	let partial = td.path().join("partial");
	fs::create_dir(&partial).expect("mkdir");
	fs::write(partial.join("y"), &big_body).expect("write");
	add::add(&local, &partial, add::FaithMode::Copy).expect("add the partial tree");
	let big = offer(&td.path().join("offers"), "big", &big_body, 0o600);

	let outcome = fetch::fetch_tree(&local, &recording_from(&remote), &hash, &[big.clone()])
		.expect("fetch succeeds");

	assert!(
		matches!(
			outcome,
			Outcome::Fetched {
				blobs_fetched: 5,
				blobs_in_stock: 1,
				ref offers_used,
				..
			} if offers_used.is_empty()
		),
		"{outcome:?}"
	);
	let after = fs::metadata(&big).expect("the offer is still at its path");
	assert_eq!(after.nlink(), 1, "the offer was linked somewhere");
	assert_eq!(after.permissions().mode() & 0o777, 0o600);
}

/// An offer on another filesystem can't be hardlinked: the whole fetch is refused, naming it, before the heap is touched.
#[test]
fn an_offer_on_another_filesystem_refuses_the_fetch() {
	let td = scratch("fetch-offer-exdev");
	let heap_dev = fs::metadata(td.path()).expect("stat").dev();
	let Some(elsewhere) = ["/dev/shm", "/run/user", "/tmp"]
		.iter()
		.map(Path::new)
		.find(|p| {
			fs::metadata(p)
				.map(|m| m.dev() != heap_dev)
				.unwrap_or(false)
		})
	else {
		eprintln!("skipping: no second filesystem found to offer a file from");
		return;
	};
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let far = tempdir::TempDir::new_in(elsewhere, "gar-test-fetch-offer-exdev-").expect("tempdir");
	let big = offer(
		far.path(),
		"big",
		&fs::read(td.path().join("specimen/nested/big")).expect("read"),
		0o644,
	);
	let rec = recording_from(&remote);

	let err = fetch::fetch_tree(&local, &rec, &hash, &[big.clone()]).expect_err("refused");

	assert!(matches!(err, fetch::FetchError::Io(_)), "{err:?}");
	assert!(
		err.to_string().contains(&big.display().to_string()),
		"{err}"
	);
	assert!(big.is_file(), "the offer is still at its path");
	assert_eq!(blob_count(&local), 0, "the heap was touched");
	assert_nothing_committed(&local, &hash);
}

/// A symlink isn't a regular file, however much its target matches: refused, and the link is left alone.
#[test]
fn a_symlink_offer_refuses_the_fetch() {
	let td = scratch("fetch-offer-symlink");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let link = td.path().join("offers/link");
	fs::create_dir(td.path().join("offers")).expect("mkdir");
	std::os::unix::fs::symlink(td.path().join("specimen/nested/big"), &link).expect("symlink");

	let err = fetch::fetch_tree(&local, &recording_from(&remote), &hash, &[link.clone()])
		.expect_err("refused");

	assert!(err.to_string().contains("not a regular file"), "{err}");
	assert!(fs::symlink_metadata(&link).expect("stat").is_symlink());
	assert_eq!(blob_count(&local), 0, "the heap was touched");
	assert_nothing_committed(&local, &hash);
}
