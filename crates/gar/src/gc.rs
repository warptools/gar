//! Garbage collection: reclaims disk space from a heap, keeping only demanded trees.
//!
//! Mark, then sweep: walk every demanded treecas entry collecting the inodes of its files,
//! remove every treecas entry that wasn't demanded,
//! then unlink every blobcas file whose inode wasn't collected.
//!
//! Gc must not run concurrently with any other use of the heap.
//! An add can link to a blob the sweep is about to judge unreferenced,
//! and a reader already inside a doomed tree has it torn down mid-walk.
//! Nothing here locks against the add race (yet); for readers there is nothing
//! a local trick could do anyway -- an open fd outlives any name we could juggle.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use crate::caswalk::for_each_shard_entry;
use crate::repo;

/// What a gc pass did, and what it couldn't: counts for reporting, and the demands that weren't in stock.
#[derive(Debug)]
pub struct Report {
	pub trees_kept: usize,
	pub trees_removed: usize,
	pub blobs_removed: usize,
	/// Demanded treehashes the heap turned out not to hold, in hex order.
	/// The gc still completes for everything else; the caller decides how loudly to fail.
	pub absent_demands: Vec<gittree::Hash>,
}

/// Removes every treecas entry not in `demands`, then every blobcas file no kept tree links to.
///
/// Inodes are compared by ino alone, not dev+ino: everything in one heap
/// is on one filesystem by construction (the hardlinks require it).
pub fn gc(repo: &repo::Repo, demands: &HashSet<gittree::Hash>) -> io::Result<Report> {
	// Mark: collect the inode of every file in every demanded tree that's actually in stock.
	let mut kept_inos: HashSet<u64> = HashSet::new();
	let mut absent_demands = Vec::new();
	for hash in demands {
		let tree = repo.tree_path(hash);
		if tree.is_dir() {
			collect_file_inos(&tree, &mut kept_inos)?;
		} else {
			absent_demands.push(hash.clone());
		}
	}
	absent_demands.sort_by_key(gittree::Hash::as_hex);

	// Sweep the treecas.
	// Removal only applies to what gc recognizes as a tree entry:
	// a directory with a canonical hash name, at shard depth.
	// Anything else in there isn't gc's to judge, and is left in place.
	let mut trees_kept = 0;
	let mut trees_removed = 0;
	for_each_shard_entry(repo.treecas_path(), &mut |ent| {
		if !ent.file_type()?.is_dir() {
			return Ok(());
		}
		let name = ent.file_name();
		let Some(hash) = name.to_str().and_then(gittree::Hash::from_hex_strict) else {
			return Ok(());
		};
		if demands.contains(&hash) {
			trees_kept += 1;
			return Ok(());
		}
		remove_tree_entry(&ent.path())?;
		trees_removed += 1;
		Ok(())
	})?;

	// Sweep the blobcas: unlink every regular file whose inode no kept tree links to.
	// No name checks here; inode identity is the whole judgement.
	let mut blobs_removed = 0;
	for_each_shard_entry(repo.blobcas_path(), &mut |ent| {
		if !ent.file_type()?.is_file() {
			return Ok(());
		}
		if kept_inos.contains(&ent.metadata()?.ino()) {
			return Ok(());
		}
		fs::remove_file(ent.path())?;
		blobs_removed += 1;
		Ok(())
	})?;

	Ok(Report {
		trees_kept,
		trees_removed,
		blobs_removed,
		absent_demands,
	})
}

/// Removes one treecas entry: renamed to a dot-prefixed temp in its shard dir first,
/// then deleted.  What the rename buys is a crash property, scoped to lookups by name:
/// a gc that dies mid-delete leaves a dot-temp, never a half-deleted tree squatting
/// at a valid CAS name -- a missing entry is just "not in stock", but a partial one
/// at its CAS name is corruption.  It buys nothing for a reader already holding
/// the tree open; that's part of why gc must not run concurrently with use.
fn remove_tree_entry(entry: &Path) -> io::Result<()> {
	let shard_dir = entry
		.parent()
		.expect("a shard entry path is at least three levels deep");
	let name = entry
		.file_name()
		.and_then(|n| n.to_str())
		.expect("a tree entry name was just parsed as hex");
	// The pid keeps two temp names apart in the only multi-writer case that should exist:
	// a previous gc crashed here, and a later one is now removing the same entry.
	let temp = shard_dir.join(format!(".gc-{}-{}", std::process::id(), name));
	fs::rename(entry, &temp)?;
	fs::remove_dir_all(&temp)
}

/// Collects the inode of every regular file under `dir`, recursively.
///
/// Symlinks aren't followed and don't count: they have no blobcas presence.
fn collect_file_inos(dir: &Path, into: &mut HashSet<u64>) -> io::Result<()> {
	for ent in fs::read_dir(dir)? {
		let ent = ent?;
		let ft = ent.file_type()?;
		if ft.is_dir() {
			collect_file_inos(&ent.path(), into)?;
		} else if ft.is_file() {
			into.insert(ent.metadata()?.ino());
		}
	}
	Ok(())
}
