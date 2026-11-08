//! Backfilling index files for trees already in a heap:
//! trees stored with indexing switched off, or by a gar that predates indexes.
//!
//! A tree that already has an index file is skipped without being read or re-hashed:
//! backfill fills gaps, and checking existing indexes against their trees is a different job.
//! A tree without one is walked and hashed from disk to generate its index,
//! and the index lands only if the tree recomputes to its own name.
//! A tree that doesn't is corrupt; it gets no index, and the rest of the work still goes on.

use std::fs;
use std::io;

use crate::caswalk::for_each_shard_entry;
use crate::index_landing;
use crate::repo;
use crate::treeidx;

/// What a backfill pass did, and what it couldn't.
///
/// The work completes for every tree it can; the failure lists say what was left out,
/// and the caller decides how loudly to fail.
#[derive(Debug, Default)]
pub struct Report {
	/// Trees that got a newly written index.
	pub indexed: usize,
	/// Trees skipped because an index file was already present.
	pub already_indexed: usize,
	/// Trees whose content doesn't hash to their name, in hex order of name.  None of these got an index.
	pub corrupt: Vec<CorruptTree>,
	/// Named trees the heap doesn't hold, in hex order.  Always empty from `index_all`.
	pub not_in_stock: Vec<gittree::Hash>,
}

/// A treecas entry whose content recomputes to a different hash than the name it's stored under.
#[derive(Debug, PartialEq, Eq)]
pub struct CorruptTree {
	pub name: gittree::Hash,
	pub computed: gittree::Hash,
}

/// Writes the missing index of every tree in the heap.
///
/// A tree is what gc and repair recognize as one: a directory wearing a canonical hash name at shard depth.
/// Anything else in the treecas is left alone.
pub fn index_all(repo: &repo::Repo) -> io::Result<Report> {
	let mut report = Report::default();
	for_each_shard_entry(repo.treecas_path(), &mut |ent| {
		if !ent.file_type()?.is_dir() {
			return Ok(());
		}
		let name = ent.file_name();
		let Some(hash) = name.to_str().and_then(gittree::Hash::from_hex_strict) else {
			return Ok(());
		};
		index_tree(repo, &hash, &mut report)
	})?;
	report.corrupt.sort_by_key(|c| c.name.as_hex());
	Ok(report)
}

/// Writes the missing index of each named tree.
///
/// A named tree the heap doesn't hold is noted in the report, and the rest are still processed.
pub fn index_named(repo: &repo::Repo, hashes: &[gittree::Hash]) -> io::Result<Report> {
	let mut report = Report::default();
	for hash in hashes {
		// Presence means the entry directory exists, the same check `gar path` makes.
		if repo.tree_path(hash).is_dir() {
			index_tree(repo, hash, &mut report)?;
		} else {
			report.not_in_stock.push(hash.clone());
		}
	}
	report.corrupt.sort_by_key(|c| c.name.as_hex());
	report.not_in_stock.sort_by_key(gittree::Hash::as_hex);
	Ok(report)
}

/// Backfills one tree that's in stock, recording the outcome in `report`.
fn index_tree(repo: &repo::Repo, hash: &gittree::Hash, report: &mut Report) -> io::Result<()> {
	// Anything at the index path counts as present: it isn't this pass's to judge.
	match fs::symlink_metadata(repo.index_path(hash)) {
		Ok(_) => {
			report.already_indexed += 1;
			return Ok(());
		},
		Err(e) if e.kind() == io::ErrorKind::NotFound => {},
		Err(e) => return Err(e),
	}
	let idx = match treeidx::generate(&repo.tree_path(hash)) {
		Ok(idx) => idx,
		Err(treeidx::GenerateError::Io(e)) => {
			return Err(io::Error::new(
				e.kind(),
				format!("indexing tree {}: {e}", hash.as_hex()),
			));
		},
		// In practice, a path too long for the format's length field.
		Err(treeidx::GenerateError::Refused(r)) => {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				format!("cannot index tree {}: {r}", hash.as_hex()),
			));
		},
	};
	if idx.root_hash() != hash {
		report.corrupt.push(CorruptTree {
			name: hash.clone(),
			computed: idx.root_hash().clone(),
		});
		return Ok(());
	}
	index_landing::land_index(repo, hash, &treeidx::serialize(&idx))?;
	report.indexed += 1;
	Ok(())
}
