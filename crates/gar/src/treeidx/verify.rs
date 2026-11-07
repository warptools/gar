//! Verification: the structural rules of a treeidx, and the treehash arithmetic.
//!
//! Both happen in one preorder pass, because they need the same thing:
//! a stack of the directories still open at each point in the entry list.
//! Each open directory accumulates its children's tree-object lines as they go by,
//! and when its listing ends, its hash is finished and checked against its claim.

use std::collections::HashSet;

use super::entry::{Entry, Kind, TreeIdx};
use super::format;
use super::refusal::{Reason, Refusal};

/// Checks that `entries` describe one tree canonically and consistently, and wraps them up as a `TreeIdx`.
///
/// Refuses at the first problem found, which is any of:
/// a first entry that isn't the root directory;
/// a malformed path, or one too long for the format;
/// a repeated path;
/// an entry not directly inside the innermost open directory (as preorder requires);
/// siblings not in strictly increasing gitwise order;
/// a symlink target that can't exist on disk (empty, or containing NUL);
/// or any directory -- the root included -- whose claimed hash differs from what its children recompute to.
pub fn verify(entries: Vec<Entry>) -> Result<TreeIdx, Refusal> {
	let root_hash = check(&entries)?;
	Ok(TreeIdx {
		entries,
		root_hash,
	})
}

/// A directory whose listing is in progress.
struct OpenDir<'a> {
	ordinal: usize,
	path: &'a [u8],
	claimed: &'a gittree::Hash,
	acc: gittree::TreeHashAccumulator,
	/// The gitwise sort key of the most recent child, which the next child must exceed.
	last_key: Option<Vec<u8>>,
}

fn refusal(ordinal: usize, entry: &Entry, reason: Reason) -> Refusal {
	Refusal {
		entry: ordinal,
		offset: None,
		path: Some(format::decorated_path(&entry.path, entry.kind.is_dir())),
		reason,
	}
}

fn open_dir<'a>(ordinal: usize, path: &'a [u8], claimed: &'a gittree::Hash) -> OpenDir<'a> {
	OpenDir {
		ordinal,
		path,
		claimed,
		acc: gittree::TreeHashAccumulator::new(0),
		last_key: None,
	}
}

/// Finishes a directory's hash and holds it to its claim.
fn close_dir(dir: OpenDir) -> Result<gittree::Hash, Refusal> {
	let computed = dir.acc.finish();
	if computed != *dir.claimed {
		return Err(Refusal {
			entry: dir.ordinal,
			offset: None,
			path: Some(format::decorated_path(dir.path, true)),
			reason: Reason::HashMismatch {
				claimed: dir.claimed.clone(),
				computed,
			},
		});
	}
	Ok(computed)
}

/// Closes a directory whose listing has ended, adding its hash to its parent's accumulator.
/// The parent is the innermost directory still open: the top of `nested`, or else the root.
fn close_into_parent<'a>(
	dir: OpenDir<'a>,
	nested: &mut [OpenDir<'a>],
	root: &mut OpenDir<'a>,
) -> Result<(), Refusal> {
	let name = split_parent(dir.path).1;
	let hash = close_dir(dir)?;
	nested
		.last_mut()
		.unwrap_or(root)
		.acc
		.append_dir(name, &hash);
	Ok(())
}

/// Splits a non-root bare path into its parent's bare path and its own name.
fn split_parent(path: &[u8]) -> (&[u8], &[u8]) {
	match path.iter().rposition(|&b| b == b'/') {
		Some(i) => (&path[..i], &path[i + 1..]),
		None => (b"", path),
	}
}

fn path_segments_ok(path: &[u8]) -> bool {
	path.split(|&b| b == b'/')
		.all(|seg| !seg.is_empty() && seg != b"." && seg != b".." && !seg.contains(&0))
}

fn check(entries: &[Entry]) -> Result<gittree::Hash, Refusal> {
	let mut root = match entries.first() {
		Some(Entry {
			path,
			kind: Kind::Dir {
				hash,
			},
		}) if path.is_empty() => open_dir(1, path, hash),
		first => {
			return Err(Refusal {
				entry: 1,
				offset: None,
				path: first.map(|e| format::decorated_path(&e.path, e.kind.is_dir())),
				reason: Reason::MissingRoot,
			});
		},
	};
	// The open directories below the root, outermost first.
	let mut nested: Vec<OpenDir> = Vec::new();
	let mut seen: HashSet<&[u8]> = HashSet::from([&b""[..]]);

	for (i, entry) in entries.iter().enumerate().skip(1) {
		let ordinal = i + 1;
		let is_dir = entry.kind.is_dir();
		// The duplicate check comes first so that a second root reads as a duplicate,
		// rather than as the empty path it also is.
		if !seen.insert(&entry.path) {
			return Err(refusal(ordinal, entry, Reason::DuplicatePath));
		}
		if !path_segments_ok(&entry.path) {
			return Err(refusal(ordinal, entry, Reason::BadPathSegment));
		}
		if format::decorated_path(&entry.path, is_dir).len() > format::MAX_PATH_LEN {
			return Err(refusal(ordinal, entry, Reason::PathTooLong));
		}

		// Preorder means the parent is open, and every directory opened since it has ended.
		// Openness is checked before closing anything, so an orphan is reported as itself,
		// rather than as the hash mismatch that closing its would-be ancestors early would cause.
		let (parent, name) = split_parent(&entry.path);
		if !parent.is_empty() && !nested.iter().any(|dir| dir.path == parent) {
			return Err(refusal(ordinal, entry, Reason::Orphan));
		}
		while let Some(dir) = nested.pop_if(|dir| dir.path != parent) {
			close_into_parent(dir, &mut nested, &mut root)?;
		}
		let top = nested.last_mut().unwrap_or(&mut root);

		let key = gittree::gitwise_sort_key(name, is_dir);
		if top.last_key.as_ref().is_some_and(|last| *last >= key) {
			return Err(refusal(ordinal, entry, Reason::OutOfOrder));
		}
		top.last_key = Some(key);

		match &entry.kind {
			Kind::File {
				executable: false,
				hash,
				..
			} => top.acc.append_file(name, hash),
			Kind::File {
				executable: true,
				hash,
				..
			} => top.acc.append_executable(name, hash),
			Kind::Symlink {
				target,
			} => {
				if target.is_empty() || target.contains(&0) {
					return Err(refusal(ordinal, entry, Reason::BadSymlinkTarget));
				}
				let hash = gittree::hash_of_stream(&mut &target[..], target.len() as u64)
					.expect("hashing a slice of its own stated length cannot fail");
				top.acc.append_symlink(name, &hash);
			},
			Kind::Dir {
				hash,
			} => nested.push(open_dir(ordinal, &entry.path, hash)),
		}
	}

	while let Some(dir) = nested.pop() {
		close_into_parent(dir, &mut nested, &mut root)?;
	}
	close_dir(root)
}
