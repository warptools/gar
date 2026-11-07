//! Walking the sharded CAS directories: the layout traversal shared by gc and repair.

use std::fs;
use std::io;
use std::path::Path;

/// Calls `f` with each entry sitting at shard depth (two directory levels) under `cas_root`.
///
/// Dot-prefixed names are skipped at every level: those are staging and temp names
/// (`.wiptree-*` dirs from in-flight adds, `.gc-*` removal temps), never CAS content.
/// Non-directories at the shard levels aren't shard dirs, and are skipped likewise.
pub(crate) fn for_each_shard_entry(
	cas_root: &Path,
	f: &mut impl FnMut(&fs::DirEntry) -> io::Result<()>,
) -> io::Result<()> {
	for outer in fs::read_dir(cas_root)? {
		let outer = outer?;
		if outer.file_name().as_encoded_bytes().starts_with(b".") || !outer.file_type()?.is_dir() {
			continue;
		}
		for inner in fs::read_dir(outer.path())? {
			let inner = inner?;
			if inner.file_name().as_encoded_bytes().starts_with(b".")
				|| !inner.file_type()?.is_dir()
			{
				continue;
			}
			for ent in fs::read_dir(inner.path())? {
				let ent = ent?;
				if ent.file_name().as_encoded_bytes().starts_with(b".") {
					continue;
				}
				f(&ent)?;
			}
		}
	}
	Ok(())
}
