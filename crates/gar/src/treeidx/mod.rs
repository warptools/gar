//! Treeidx files: a deterministic, self-contained, line-oriented index of one tree.
//!
//! An index lists every path in a tree with its mode, size, and hash
//! (or, for a symlink, its target), which is enough to recompute the tree's root hash
//! by pure arithmetic, with no file IO.  That lets a reader check a remote's claims
//! about a tree before fetching any of its content.
//!
//! The byte format is specified in README_formats.md.
//! Every `TreeIdx` value is verified: its structure is canonical,
//! and every directory hash it claims recomputes from its children.
//! Parsing and serializing are therefore exact inverses, with exactly one byte sequence per tree.

mod entry;
mod format;
mod generate;
mod parse;
mod refusal;
mod serialize;
mod verify;

#[cfg(test)]
mod generate_test;
#[cfg(test)]
mod parse_test;
#[cfg(test)]
mod testutil;
#[cfg(test)]
mod verify_test;

pub use entry::{Entry, Kind, TreeIdx};
pub use generate::{GenerateError, generate};
pub use parse::parse;
pub use refusal::{Reason, Refusal};
pub use serialize::serialize;
pub use verify::verify;
