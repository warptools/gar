//! Import: reads a tar stream into the heap, storing it as `add` would.
//!
//! The stream is untrusted input, and everything about the read is fail-closed:
//! entry types outside {file, directory, symlink} are refused by name,
//! and so is any entry whose path is absolute or tries to `..` its way out.
//! The tar crate's own `unpack_in` would merely *skip* such paths, which is not refusal;
//! the checks here happen first, so nothing is quietly dropped or relocated.
//!
//! Entries are unpacked into a staging directory inside the repo (dot-prefixed,
//! cleaned up on drop, same filesystem as the CAS), with modes normalized to the heap's
//! guarantees as they land.  The staged tree then goes through the ordinary add machinery,
//! and the hash that comes back is computed from what was actually stored:
//! trust the arithmetic, not the sender.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};

use crate::add;
use crate::repo;

/// Why an import stopped: the stream was unacceptable, or the machinery underneath failed.
///
/// `Refused` is a judgement about the input (the named entry broke the rules);
/// `Io` is everything else, including a tar stream too mangled to parse.
#[derive(Debug)]
pub enum ImportError {
	Io(io::Error),
	Refused {
		entry: String,
		reason: String,
	},
}

impl From<io::Error> for ImportError {
	fn from(e: io::Error) -> Self {
		ImportError::Io(e)
	}
}

impl fmt::Display for ImportError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			ImportError::Io(e) => e.fmt(f),
			ImportError::Refused {
				entry,
				reason,
			} => {
				write!(f, "refusing entry {entry:?}: {reason}")
			},
		}
	}
}

/// Reads one tar stream from `reader` and stores it, returning the treehash of what landed.
///
/// The returned hash is computed from the stored bytes; a caller expecting a particular
/// tree compares afterward.  Either way the stored tree stays in the heap:
/// a CAS is append-only, and removal is gc's job, not import's.
pub fn import(repo: &repo::Repo, reader: impl io::Read) -> Result<gittree::Hash, ImportError> {
	let staging = tempdir::TempDir::new_in(repo.repo_path(), ".import-")?;
	unpack_normalized(reader, staging.path())?;
	// The staging tree is fresh, ours alone, and on the CAS's filesystem,
	// so linking the staged files straight into the blobcas is sound, and skips a second copy.
	Ok(add::add(
		repo,
		staging.path(),
		add::FaithMode::LinkOriginals,
	)?)
}

/// Unpacks the stream into `staging`, normalizing modes to the heap's as entries land:
/// 0444 or 0555 for files by the header's user-exec bit, 0755 for directories.
fn unpack_normalized(reader: impl io::Read, staging: &Path) -> Result<(), ImportError> {
	let mut archive = tar::Archive::new(reader);
	for entry in archive.entries()? {
		let mut entry = entry?;
		let kind = entry.header().entry_type();
		match kind {
			tar::EntryType::Regular | tar::EntryType::Directory | tar::EntryType::Symlink => {},
			_ => {
				return Err(refusal(
					&entry,
					format!("gar transports files, directories, and symlinks; this is {kind:?}"),
				));
			},
		}
		let rel = checked_rel_path(&entry)?;
		if rel.as_os_str().is_empty() {
			// A bare "./" naming the root: common in foreign tars, and already exists here.
			// (checked_rel_path only lets this through for a directory entry.)
			continue;
		}
		let executable = gittree::mode_is_executable(entry.header().mode()?);
		if !entry.unpack_in(staging)? {
			// unpack_in only skips paths the check above already refused, so this arm
			// shouldn't be reachable -- but a skip surfacing as success would be an import
			// that quietly stored less than it was sent, so it gets refused loudly instead.
			return Err(refusal(
				&entry,
				"the tar unpacker skipped this entry".into(),
			));
		}
		let mode = match kind {
			tar::EntryType::Regular if executable => 0o555,
			tar::EntryType::Regular => 0o444,
			tar::EntryType::Directory => 0o755,
			// Symlinks don't carry permission bits on linux.
			_ => continue,
		};
		fs::set_permissions(staging.join(&rel), fs::Permissions::from_mode(mode))?;
	}
	Ok(())
}

/// The entry's path with any `./` spellings dropped -- or a refusal if it's absolute,
/// contains `..`, or is empty while claiming to be a file or symlink.
///
/// An empty result (legal only for a directory entry) names the staging root itself.
fn checked_rel_path(entry: &tar::Entry<impl io::Read>) -> Result<PathBuf, ImportError> {
	let path = entry.path()?;
	let mut clean = PathBuf::new();
	for comp in path.components() {
		match comp {
			Component::Normal(part) => clean.push(part),
			// "./" prefixes are a harmless spelling many tar writers use.
			Component::CurDir => {},
			Component::RootDir | Component::Prefix(_) => {
				return Err(refusal(entry, "absolute paths are refused".into()));
			},
			Component::ParentDir => {
				return Err(refusal(
					entry,
					"`..` paths are refused (they could escape the import)".into(),
				));
			},
		}
	}
	if clean.as_os_str().is_empty() && entry.header().entry_type() != tar::EntryType::Directory {
		return Err(refusal(
			entry,
			"a file or symlink entry must have a name".into(),
		));
	}
	Ok(clean)
}

/// A `Refused` error naming the entry as the stream spelled it.
fn refusal(entry: &tar::Entry<impl io::Read>, reason: String) -> ImportError {
	ImportError::Refused {
		entry: String::from_utf8_lossy(&entry.path_bytes()).into_owned(),
		reason,
	}
}
