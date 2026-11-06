//! Export: writes a treecas entry as a tar stream.
//!
//! The stream is deterministic: the same tree exports as byte-identical tar, every time, everywhere.
//! Entry order is the git-wise walk order (the same order tree hashing visits),
//! and every header field that could vary is pinned: mtime 0, uid/gid 0, no uname/gname,
//! and modes exactly as the heap guarantees them.
//! The normative description of the stream lives in README_formats.md (the tar transport section).

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use crate::repo;

/// Writes the tree named by `hash` to `out` as a tar stream.
///
/// The caller owns buffering and flushing of `out`; this writes the archive and its
/// trailing end-of-archive blocks, nothing more.  A tree not in the heap is a `NotFound` error,
/// though callers wanting a friendly diagnostic should check `Repo::tree_path` themselves first.
pub fn export(repo: &repo::Repo, hash: &gittree::Hash, out: impl io::Write) -> io::Result<()> {
	let tree = repo.tree_path(hash);
	if !tree.is_dir() {
		return Err(io::Error::new(
			io::ErrorKind::NotFound,
			format!("tree {} is not in this heap", hash.as_hex()),
		));
	}
	let mut builder = tar::Builder::new(out);
	export_dir(&tree, Path::new(""), &mut builder)?;
	// finish writes the end-of-archive blocks; without it the stream is silently truncated.
	builder.finish()
}

/// A header with everything pinned except what the caller sets per entry: type, mode, size, path.
///
/// GNU-flavored headers are what the tar crate itself defaults to; `new_gnu` is all zeroes
/// plus the magic, so no ambient metadata can leak in -- but say the pinning anyway,
/// since determinism here is a promise, not a happenstance.
fn pinned_header() -> tar::Header {
	let mut header = tar::Header::new_gnu();
	header.set_uid(0);
	header.set_gid(0);
	header.set_mtime(0);
	header
}

/// Exports the children of one directory, recursively, in git-wise order.
///
/// `scan_dir` is the directory on disk; `rel` is its name in the stream (empty at the root:
/// the tree's root directory is the stream itself, and gets no entry).
fn export_dir(
	scan_dir: &Path,
	rel: &Path,
	builder: &mut tar::Builder<impl io::Write>,
) -> io::Result<()> {
	let mut entries = fs::read_dir(scan_dir)?.collect::<Result<Vec<_>, io::Error>>()?;
	gittree::sort_dirents_gitwise(&mut entries);
	for ent in entries {
		let ft = ent.file_type()?;
		let rel_path = rel.join(ent.file_name());
		if ft.is_file() {
			let mut file = fs::File::open(ent.path())?;
			let meta = file.metadata()?;
			let executable = gittree::mode_is_executable(meta.permissions().mode());
			let mut header = pinned_header();
			header.set_entry_type(tar::EntryType::Regular);
			header.set_mode(if executable { 0o555 } else { 0o444 });
			header.set_size(meta.size());
			builder.append_data(&mut header, &rel_path, &mut file)?;
		} else if ft.is_symlink() {
			let target = fs::read_link(ent.path())?;
			let mut header = pinned_header();
			header.set_entry_type(tar::EntryType::Symlink);
			// Symlink permission bits are meaningless on linux; 0o777 is what tar tools conventionally write.
			header.set_mode(0o777);
			header.set_size(0);
			builder.append_link(&mut header, &rel_path, &target)?;
		} else if ft.is_dir() {
			// Directory names carry the conventional trailing slash, for the benefit of foreign readers.
			let mut name = rel_path.clone().into_os_string();
			name.push("/");
			let mut header = pinned_header();
			header.set_entry_type(tar::EntryType::Directory);
			header.set_mode(0o755);
			header.set_size(0);
			builder.append_data(&mut header, Path::new(&name), io::empty())?;
			export_dir(&ent.path(), &rel_path, builder)?;
		} else {
			// A treecas entry holds only files, dirs, and symlinks by construction;
			// anything else means the heap is corrupt, and a corrupt heap must not export quietly.
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				format!("treecas entry contains a non-exportable file type at {rel_path:?}"),
			));
		}
	}
	Ok(())
}
