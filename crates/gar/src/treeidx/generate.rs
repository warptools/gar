//! The disk-walk generator: a materialized tree on disk to its verified index.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::entry::{Entry, Kind, TreeIdx};
use super::refusal::Refusal;
use super::verify::verify;

/// Why generating an index stopped: the walk failed, or the tree can't be expressed as a treeidx.
#[derive(Debug)]
pub enum GenerateError {
	Io(io::Error),
	/// The tree on disk breaks a treeidx rule (in practice: a path too long for the length field).
	Refused(Refusal),
}

impl From<io::Error> for GenerateError {
	fn from(e: io::Error) -> Self {
		GenerateError::Io(e)
	}
}

impl fmt::Display for GenerateError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			GenerateError::Io(e) => e.fmt(f),
			GenerateError::Refused(r) => r.fmt(f),
		}
	}
}

/// Walks the directory at `root` and builds its index, hashing every file's content from disk.
///
/// Meant for materialized trees (treecas entries), which hold only files, directories, and symlinks;
/// any other file type is an `InvalidData` error.
/// Nothing is skipped: a `.gar` directory inside `root` is indexed like any other.
/// The root hash agrees with `gittree::hash_of_path` on the same directory.
pub fn generate(root: &Path) -> Result<TreeIdx, GenerateError> {
	let mut walk = Walk {
		entries: Vec::new(),
		// Overwritten in every directory's slot before the walk returns.
		placeholder: gittree::Hash::from_hex([b'0'; 64]).expect("64 zeros are valid hex"),
	};
	walk.dir(root, Vec::new())?;
	verify(walk.entries).map_err(GenerateError::Refused)
}

struct Walk {
	entries: Vec<Entry>,
	placeholder: gittree::Hash,
}

impl Walk {
	/// Indexes one directory and everything under it, returning its treehash.
	///
	/// Entries come out in preorder, but a directory's hash is only known once its children are done,
	/// so its entry is pushed first with a placeholder hash, then filled in.
	fn dir(&mut self, disk_path: &Path, bare: Vec<u8>) -> io::Result<gittree::Hash> {
		let slot = self.entries.len();
		self.entries.push(Entry {
			path: bare.clone(),
			kind: Kind::Dir {
				hash: self.placeholder.clone(),
			},
		});

		let mut dirents = fs::read_dir(disk_path)?.collect::<io::Result<Vec<_>>>()?;
		gittree::sort_dirents_gitwise(&mut dirents);
		let mut acc = gittree::TreeHashAccumulator::new(dirents.len());
		for ent in dirents {
			let name = ent.file_name();
			let name = name.as_encoded_bytes();
			let child = child_path(&bare, name);
			let ft = ent.file_type()?;
			if ft.is_file() {
				let mut file = fs::File::open(ent.path())?;
				let meta = file.metadata()?;
				let hash = gittree::hash_of_stream(&mut file, meta.size())?;
				let executable = gittree::mode_is_executable(meta.permissions().mode());
				if executable {
					acc.append_executable(name, &hash);
				} else {
					acc.append_file(name, &hash);
				}
				self.entries.push(Entry {
					path: child,
					kind: Kind::File {
						executable,
						size: meta.size(),
						hash,
					},
				});
			} else if ft.is_symlink() {
				let target = fs::read_link(ent.path())?
					.into_os_string()
					.into_encoded_bytes();
				let hash = gittree::hash_of_stream(&mut &target[..], target.len() as u64)?;
				acc.append_symlink(name, &hash);
				self.entries.push(Entry {
					path: child,
					kind: Kind::Symlink {
						target,
					},
				});
			} else if ft.is_dir() {
				let hash = self.dir(&ent.path(), child)?;
				acc.append_dir(name, &hash);
			} else {
				return Err(io::Error::new(
					io::ErrorKind::InvalidData,
					format!("not a file, directory, or symlink: {:?}", ent.path()),
				));
			}
		}

		let hash = acc.finish();
		self.entries[slot].kind = Kind::Dir {
			hash: hash.clone(),
		};
		Ok(hash)
	}
}

fn child_path(parent: &[u8], name: &[u8]) -> Vec<u8> {
	if parent.is_empty() {
		return name.to_vec();
	}
	let mut out = Vec::with_capacity(parent.len() + 1 + name.len());
	out.extend_from_slice(parent);
	out.push(b'/');
	out.extend_from_slice(name);
	out
}
