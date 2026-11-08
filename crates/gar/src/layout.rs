//! The heap's layout, spelled once: where every entry sits, relative to the heap's own directory.
//!
//! `Repo` joins these onto its path on disk; fetch transports join them onto a remote base.
//! The layout is specified in README_formats.md (the sharding and blobcas sections).

use std::fmt;
use std::path::Path;

pub const BLOBCAS: &str = "blobcas";
pub const TREECAS: &str = "treecas";
pub const TREEIDX: &str = "treeidx";
pub const QUARANTINE: &str = "quarantine";

/// The name suffix that marks a blob as the executable variant.
const EXEC_SUFFIX: &str = "-x";

/// A path relative to a heap's own directory (the `.gar` dir), `/`-separated.
///
/// Only this module makes these, so the contents are always one of the layout's own spellings:
/// a CAS root name, two shard levels, and an entry name, in lowercase hex plus `-x` at most.
/// That makes them safe to join onto a filesystem path or a URL as-is, with no escaping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelPath(String);

impl RelPath {
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl AsRef<Path> for RelPath {
	fn as_ref(&self) -> &Path {
		Path::new(&self.0)
	}
}

impl fmt::Display for RelPath {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

/// Where a blob lives: `blobcas/{ab}/{cd}/{hash}`, plus `-x` for the executable variant.
pub fn blob(hash: &gittree::Hash, executable: bool) -> RelPath {
	let suffix = if executable { EXEC_SUFFIX } else { "" };
	sharded(BLOBCAS, hash, suffix)
}

/// Where a tree lives: `treecas/{ab}/{cd}/{hash}`.
pub fn tree(hash: &gittree::Hash) -> RelPath {
	sharded(TREECAS, hash, "")
}

/// Where a tree's index file lives: `treeidx/{ab}/{cd}/{hash}`, named by the treehash, sharded like the treecas.
pub fn index(hash: &gittree::Hash) -> RelPath {
	sharded(TREEIDX, hash, "")
}

/// Parses a blobcas entry name: 64 lowercase hex characters, with an optional `-x` suffix
/// marking the executable variant.  Anything else is not a blob name.
pub fn parse_blob_name(name: &str) -> Option<(gittree::Hash, bool)> {
	let (hex, exec) = match name.strip_suffix(EXEC_SUFFIX) {
		Some(hex) => (hex, true),
		None => (name, false),
	};
	gittree::Hash::from_hex_strict(hex).map(|h| (h, exec))
}

/// `{root}/{ab}/{cd}/{hash}{suffix}`, from the first four hex chars of the hash.
///
/// Sharding keeps any one directory from accumulating millions of entries.
fn sharded(root: &str, hash: &gittree::Hash, suffix: &str) -> RelPath {
	let hex = hash.as_hex();
	RelPath(format!(
		"{root}/{}/{}/{hex}{suffix}",
		&hex[0..2],
		&hex[2..4]
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn some_hash() -> gittree::Hash {
		gittree::Hash::from_hex("2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce")
			.expect("valid hex")
	}

	/// The spellings from README_formats.md's sharding section, verbatim.
	#[test]
	fn spellings_match_the_spec() {
		let h = some_hash();
		let hex = "2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce";
		assert_eq!(blob(&h, false).as_str(), format!("blobcas/29/09/{hex}"));
		assert_eq!(blob(&h, true).as_str(), format!("blobcas/29/09/{hex}-x"));
		assert_eq!(tree(&h).as_str(), format!("treecas/29/09/{hex}"));
		assert_eq!(index(&h).as_str(), format!("treeidx/29/09/{hex}"));
	}

	#[test]
	fn blob_names_round_trip() {
		let h = some_hash();
		for exec in [false, true] {
			let rel = blob(&h, exec);
			let name = rel.as_str().rsplit('/').next().expect("has a last segment");
			assert_eq!(parse_blob_name(name), Some((h.clone(), exec)));
		}
		assert_eq!(parse_blob_name("2909"), None);
		assert_eq!(parse_blob_name(&format!("{}-y", h.as_hex())), None);
	}
}
