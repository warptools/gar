//! Why a treeidx was refused, and where.

use std::fmt;

/// A treeidx (as bytes, or as entries) that isn't acceptable, and the first place that showed it.
///
/// Refusal is all-or-nothing: no part of a refused index should be acted on.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
	/// The 1-based ordinal of the offending entry; 0 means the header.
	pub entry: usize,
	/// The byte offset in the input where the problem was found, when bytes were being parsed.
	pub offset: Option<usize>,
	/// The offending entry's path as the format spells it, when known.
	pub path: Option<Vec<u8>>,
	pub reason: Reason,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reason {
	// Byte-level problems, which only parsing can find.
	BadHeader,
	Truncated,
	BadLengthField,
	MissingSpace,
	MissingNewline,
	BadMode,
	BadSizeField,
	BadHash,
	PathNotDotted,
	DirPathWithoutSlash,
	NonDirPathWithSlash,

	// Problems with the entries themselves, whatever form they arrived in.
	MissingRoot,
	BadPathSegment,
	PathTooLong,
	BadSymlinkTarget,
	DuplicatePath,
	Orphan,
	OutOfOrder,

	// The arithmetic: a directory's claimed hash isn't what its children recompute to.
	HashMismatch {
		claimed: gittree::Hash,
		computed: gittree::Hash,
	},
}

impl fmt::Display for Reason {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Reason::BadHeader => write!(f, "the file must begin with the line \"treeidx v1\""),
			Reason::Truncated => write!(f, "the input ends partway through an entry"),
			Reason::BadLengthField => write!(
				f,
				"the path length field must be exactly 5 bytes: a decimal number without leading zeros, right-aligned with spaces"
			),
			Reason::MissingSpace => write!(f, "expected a single space separating fields"),
			Reason::MissingNewline => write!(f, "expected a newline ending the entry"),
			Reason::BadMode => write!(
				f,
				"the mode must be one of 100644, 100755, 040000, or 120000"
			),
			Reason::BadSizeField => write!(
				f,
				"the size field must be \"-\" for a directory, and otherwise a decimal number without leading zeros"
			),
			Reason::BadHash => write!(f, "a hash must be exactly 64 lowercase hex characters"),
			Reason::PathNotDotted => write!(f, "paths must begin with \"./\""),
			Reason::DirPathWithoutSlash => write!(f, "directory paths must end with \"/\""),
			Reason::NonDirPathWithSlash => write!(f, "only directory paths may end with \"/\""),
			Reason::MissingRoot => write!(f, "the first entry must be the root directory, \"./\""),
			Reason::BadPathSegment => write!(
				f,
				"path segments must be non-empty, must not be \".\" or \"..\", and must not contain NUL"
			),
			Reason::PathTooLong => write!(
				f,
				"the path is longer than the format's limit of 99999 bytes"
			),
			Reason::BadSymlinkTarget => write!(
				f,
				"symlink targets must be non-empty and must not contain NUL"
			),
			Reason::DuplicatePath => write!(f, "this path already appeared in an earlier entry"),
			Reason::Orphan => write!(
				f,
				"the parent directory is not open at this point (no entry for it precedes this one, it isn't a directory, or its listing already ended)"
			),
			Reason::OutOfOrder => write!(f, "entries in a directory must be in git's sort order"),
			Reason::HashMismatch {
				claimed,
				computed,
			} => write!(
				f,
				"the directory claims hash {}, but its entries hash to {}",
				claimed.as_hex(),
				computed.as_hex()
			),
		}
	}
}

impl fmt::Display for Refusal {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.entry == 0 {
			write!(f, "treeidx header")?;
		} else {
			write!(f, "treeidx entry {}", self.entry)?;
		}
		if let Some(offset) = self.offset {
			write!(f, " at byte {offset}")?;
		}
		if let Some(path) = &self.path {
			write!(f, " (\"{}\")", path.escape_ascii())?;
		}
		write!(f, ": {}", self.reason)
	}
}
