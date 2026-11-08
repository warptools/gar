//! The in-memory form of a treeidx: entries, and the verified list of them.

/// One path in a tree, as a treeidx describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
	/// The path relative to the tree root, as raw bytes, `/`-separated.
	///
	/// This is the bare path: no `./` prefix and no trailing `/` on directories
	/// (the serialized form adds both).  The root directory's path is empty.
	/// Bytes rather than a string, because filenames need not be UTF-8.
	pub path: Vec<u8>,
	pub kind: Kind,
}

/// What an entry is, along with everything the tree hash needs from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
	/// A regular file: mode 100644, or 100755 when `executable`.
	File {
		executable: bool,
		/// The content's exact length in bytes.
		/// Git tree hashes don't cover sizes, so verification can't check this one;
		/// whoever reads the blob itself must.
		size: u64,
		hash: gittree::Hash,
	},
	/// A directory, and the treehash it claims.
	Dir {
		hash: gittree::Hash,
	},
	/// A symlink, carried by its target.
	/// The target's size and blobhash both follow from the target bytes, so neither is stored.
	Symlink {
		target: Vec<u8>,
	},
}

impl Kind {
	pub fn is_dir(&self) -> bool {
		matches!(self, Kind::Dir { .. })
	}
}

/// A verified treeidx: entries in canonical order, forming one coherent tree,
/// with every claimed directory hash equal to what its children recompute to.
///
/// The only ways to get one are `verify`, `parse`, and `generate`, which all check these properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeIdx {
	pub(super) entries: Vec<Entry>,
	pub(super) root_hash: gittree::Hash,
}

impl TreeIdx {
	/// The entries, root first, in preorder with gitwise sibling order.
	pub fn entries(&self) -> &[Entry] {
		&self.entries
	}

	pub fn into_entries(self) -> Vec<Entry> {
		self.entries
	}

	/// The treehash of the whole tree.  (It's also the root entry's hash.)
	pub fn root_hash(&self) -> &gittree::Hash {
		&self.root_hash
	}
}
