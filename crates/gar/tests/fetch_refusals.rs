//! Tests for `fetch::fetch_tree` refusing or failing, between heaps on disk, over the local-filesystem transport.
//!
//! Every failure is checked for what it must not leave behind: a tree, an index, staging temps,
//! or any blob that didn't verify.

mod fetch_support;

use std::fs;

use fetch_support::{
	BreaksMidStream, assert_nothing_committed, blob_count, blob_rel, heap, leftover_temps,
	location, overwrite, plant_specimen, recording_from, remote_index, remote_with_specimen,
	rewrite_remote_index, scratch, set_size,
};
use gar::add;
use gar::fetch::{self, FetchError, Rejection};
use gar::layout;
use gar::treeidx;

/// An index whose directory hash doesn't recompute is refused whole, before any blob is opened.
#[test]
fn an_index_with_a_mangled_dir_hash_is_refused_before_any_blob() {
	let td = scratch("fetch-mangled-dir");
	let (remote, hash) = remote_with_specimen(td.path());
	let local = heap(td.path(), "local");
	let nested_hex = remote_index(&remote, &hash)
		.entries()
		.iter()
		.find_map(|e| match &e.kind {
			treeidx::Kind::Dir {
				hash,
			} if e.path == b"nested" => Some(hash.as_hex()),
			_ => None,
		})
		.expect("the index has nested/");
	let text = fs::read_to_string(remote.index_path(&hash)).expect("index is utf8 here");
	assert_eq!(text.matches(&nested_hex).count(), 1);
	// The stored index is read-only: replace it, don't overwrite.
	fs::remove_file(remote.index_path(&hash)).expect("remove the stored index");
	fs::write(
		remote.index_path(&hash),
		text.replace(&nested_hex, &"a".repeat(64)),
	)
	.expect("write the mangled index");
	let rec = recording_from(&remote);

	let err = fetch::fetch_tree(&local, &rec, &hash, &[]).expect_err("a lying index");

	match err {
		FetchError::Refused {
			location: loc,
			rejection: Rejection::BadIndex(refusal),
		} => {
			assert_eq!(loc, location(&remote, &layout::index(&hash)));
			assert!(
				matches!(refusal.reason, treeidx::Reason::HashMismatch { .. }),
				"{refusal}"
			);
		},
		e => panic!("expected a BadIndex refusal, got {e:?}"),
	}
	assert_eq!(rec.opens(), vec![layout::index(&hash).as_str().to_owned()]);
	assert_eq!(blob_count(&local), 0);
	assert_nothing_committed(&local, &hash);
}

/// A sound index for some other tree, served under the asked-for name, is refused before any blob is opened.
#[test]
fn an_index_for_another_tree_is_refused_before_any_blob() {
	let td = scratch("fetch-wrong-root");
	let (remote, hash) = remote_with_specimen(td.path());
	let other_src = td.path().join("other");
	fs::create_dir(&other_src).expect("mkdir");
	fs::write(other_src.join("f"), b"other\n").expect("write");
	let other = add::add(&remote, &other_src, add::FaithMode::Copy).expect("add other");
	// The stored index is read-only: replace it, don't overwrite.
	fs::remove_file(remote.index_path(&hash)).expect("remove the stored index");
	fs::copy(remote.index_path(&other), remote.index_path(&hash)).expect("swap the index");
	let local = heap(td.path(), "local");
	let rec = recording_from(&remote);

	let err = fetch::fetch_tree(&local, &rec, &hash, &[]).expect_err("the wrong tree");

	match err {
		FetchError::Refused {
			location: loc,
			rejection: Rejection::WrongTree {
				described,
			},
		} => {
			assert_eq!(loc, location(&remote, &layout::index(&hash)));
			assert_eq!(described, other);
		},
		e => panic!("expected a WrongTree refusal, got {e:?}"),
	}
	assert_eq!(rec.opens(), vec![layout::index(&hash).as_str().to_owned()]);
	assert_eq!(blob_count(&local), 0);
	assert_nothing_committed(&local, &hash);
}

/// A blob whose bytes don't match its name is refused; blobs that verified before it stay.
#[test]
fn a_corrupt_remote_blob_is_refused() {
	let td = scratch("fetch-corrupt-blob");
	let (remote, hash) = remote_with_specimen(td.path());
	let remote_idx = remote_index(&remote, &hash);
	let bad = blob_rel(&remote_idx, "nested/run.sh");
	// Same length, so it's the hash check that has to catch it.
	overwrite(&remote.repo_path().join(&bad), b"#!/bin/sh\necho HI\n");
	let local = heap(td.path(), "local");
	let rec = recording_from(&remote);

	let err = fetch::fetch_tree(&local, &rec, &hash, &[]).expect_err("a corrupt blob");

	match err {
		FetchError::Refused {
			location: loc,
			rejection: Rejection::WrongContent {
				..
			},
		} => assert_eq!(loc, location(&remote, &bad)),
		e => panic!("expected a WrongContent refusal, got {e:?}"),
	}
	assert!(!local.repo_path().join(&bad).exists());
	// Every blob opened before the bad one verified and landed, and stays.
	let opens = rec.opens();
	let before: Vec<&String> = opens[1..]
		.iter()
		.take_while(|r| r.as_str() != bad.as_str())
		.collect();
	assert!(
		!before.is_empty(),
		"the bad blob should not be the first fetched: {opens:#?}"
	);
	for rel in &before {
		assert!(
			local.repo_path().join(rel).is_file(),
			"{rel} should have stayed"
		);
	}
	assert_eq!(blob_count(&local), before.len());
	assert_nothing_committed(&local, &hash);
}

/// A verified index naming a blob the remote doesn't hold gets its own diagnostic.
#[test]
fn a_blob_the_remote_lacks_is_its_own_error() {
	let td = scratch("fetch-blob-missing");
	let (remote, hash) = remote_with_specimen(td.path());
	let gone = blob_rel(&remote_index(&remote, &hash), "has space");
	fs::remove_file(remote.repo_path().join(&gone)).expect("remove a remote blob");
	let local = heap(td.path(), "local");

	let err =
		fetch::fetch_tree(&local, &recording_from(&remote), &hash, &[]).expect_err("missing blob");

	match &err {
		FetchError::BlobMissing {
			location: loc,
		} => assert_eq!(*loc, location(&remote, &gone)),
		e => panic!("expected BlobMissing, got {e:?}"),
	}
	assert!(
		err.to_string().contains("advertises blobs it doesn't hold"),
		"{err}"
	);
	assert_nothing_committed(&local, &hash);
}

/// No index at all: the tree isn't served there.
#[test]
fn an_absent_index_is_not_served() {
	let td = scratch("fetch-not-served");
	let (remote, hash) = remote_with_specimen(td.path());
	fs::remove_file(remote.index_path(&hash)).expect("remove the index");
	let local = heap(td.path(), "local");

	match fetch::fetch_tree(&local, &recording_from(&remote), &hash, &[]) {
		Err(FetchError::NotServed {
			location: loc,
		}) => assert_eq!(loc, location(&remote, &layout::index(&hash))),
		r => panic!("expected NotServed, got {r:?}"),
	}
	assert_nothing_committed(&local, &hash);
}

/// A body longer than the index claims is cut off one byte past the claim, and refused:
/// a remote can't make fetch read (or store) without bound.
#[test]
fn a_body_past_its_claimed_size_is_cut_off_and_refused() {
	let td = scratch("fetch-body-too-long");
	let (remote, hash) = remote_with_specimen(td.path());
	rewrite_remote_index(&remote, &hash, |entries| {
		set_size(entries, "nested/big", 10)
	});
	let big = blob_rel(&remote_index(&remote, &hash), "nested/big");
	let local = heap(td.path(), "local");
	let rec = recording_from(&remote);

	let err = fetch::fetch_tree(&local, &rec, &hash, &[]).expect_err("an overlong body");

	match err {
		FetchError::Refused {
			location: loc,
			rejection: Rejection::BodyTooLong {
				claimed: 10,
			},
		} => assert_eq!(loc, location(&remote, &big)),
		e => panic!("expected a BodyTooLong refusal, got {e:?}"),
	}
	assert_eq!(rec.bytes_read(&big), 11);
	assert!(!local.repo_path().join(&big).exists());
	assert_nothing_committed(&local, &hash);
}

/// Size claims are checked against each blob's true size, wherever that came from:
/// a fetched blob, the same blob seen again under another path, or a blob already in stock.
/// Whatever verified by hash stays; the refusal is of the index.
#[test]
fn a_size_claim_that_isnt_the_blobs_size_is_refused() {
	for (case, lie_at, stock_a_file) in [
		("fetched", "has space", false),
		("second-path", "copy_of_a", false),
		("in-stock", "a_file", true),
	] {
		let td = scratch(&format!("fetch-size-lie-{case}"));
		let (remote, hash) = remote_with_specimen(td.path());
		rewrite_remote_index(&remote, &hash, |entries| set_size(entries, lie_at, 1000));
		let local = heap(td.path(), "local");
		if stock_a_file {
			let src = plant_specimen(td.path(), "stock");
			for name in [
				"copy_of_a",
				"exec_twin",
				"has space",
				"link",
				"spaced_target",
			] {
				fs::remove_file(src.join(name)).expect("trim");
			}
			fs::remove_dir_all(src.join("nested")).expect("trim");
			add::add(&local, &src, add::FaithMode::Copy).expect("stock a_file's blob");
		}
		let rec = recording_from(&remote);

		let err = fetch::fetch_tree(&local, &rec, &hash, &[]).expect_err(case);

		match err {
			FetchError::Refused {
				location: loc,
				rejection:
					Rejection::SizeMismatch {
						path,
						claimed: 1000,
						actual: 7,
					},
			} => {
				assert_eq!(loc, location(&remote, &layout::index(&hash)), "{case}");
				assert_eq!(path, lie_at.as_bytes(), "{case}");
			},
			e => panic!("{case}: expected a SizeMismatch refusal, got {e:?}"),
		}
		if stock_a_file {
			assert_eq!(
				rec.opens().len(),
				1,
				"{case}: only the index: {:#?}",
				rec.opens()
			);
		} else {
			let lied = blob_rel(&remote_index(&remote, &hash), lie_at);
			assert!(
				local.repo_path().join(&lied).is_file(),
				"{case}: the verified blob stays"
			);
		}
		assert!(!local.tree_path(&hash).exists(), "{case}");
		assert!(!local.index_path(&hash).exists(), "{case}");
		assert_eq!(
			leftover_temps(&local),
			Vec::<std::path::PathBuf>::new(),
			"{case}"
		);
	}
}

/// A transport failure, on open or partway through a body, is an io error naming where it happened.
#[test]
fn transport_failures_name_their_location() {
	for case in ["blob-open", "blob-mid-stream", "index-mid-stream"] {
		let td = scratch(&format!("fetch-transport-{case}"));
		let (remote, hash) = remote_with_specimen(td.path());
		let broken = match case {
			"index-mid-stream" => layout::index(&hash),
			_ => blob_rel(&remote_index(&remote, &hash), "has space"),
		};
		let local = heap(td.path(), "local");
		let plain: Box<dyn fetch::Transport> =
			Box::new(fetch::LocalTransport::new(remote.repo_path()));
		let transport: Box<dyn fetch::Transport> = match case {
			"blob-open" => {
				// The local transport refuses a directory where a file should be.
				let at = remote.repo_path().join(&broken);
				fs::remove_file(&at).expect("remove the blob");
				fs::create_dir(&at).expect("put a dir there");
				plain
			},
			_ => Box::new(BreaksMidStream {
				inner: plain,
				broken: broken.clone(),
			}),
		};

		let err = fetch::fetch_tree(&local, &*transport, &hash, &[]).expect_err(case);

		assert!(
			matches!(err, FetchError::Io(_)),
			"{case}: expected Io, got {err:?}"
		);
		assert!(
			err.to_string().contains(&location(&remote, &broken)),
			"{case}: {err}"
		);
		assert!(!local.repo_path().join(&broken).exists(), "{case}");
		assert!(!local.tree_path(&hash).exists(), "{case}");
		assert_eq!(
			leftover_temps(&local),
			Vec::<std::path::PathBuf>::new(),
			"{case}"
		);
	}
}
