//! Repair: verifies the whole heap from its bytes, restores the hardlink structure,
//! and quarantines whatever is provably corrupt.
//!
//! The motivating scenario is a heap copied by a tool that breaks hardlinks
//! (and possibly mangles modes or bytes along the way): every treecas file lands on
//! its own inode, and the dedup quietly evaporates.
//!
//! Two passes, blobcas first so the treecas pass stands on verified ground:
//!
//! - Blob pass: hash every blobcas entry.  A name whose content doesn't hash to it is quarantined;
//!   a verified name has its mode normalized to what the name claims (the `-x` suffix is authoritative,
//!   since tree entries reference the suffixed name).  Verified inodes are remembered,
//!   so the tree pass never re-reads a file whose hardlink is intact.
//! - Tree pass: walk every treecas entry, rehashing exactly as `add` does.
//!   A file on an unrecognized inode gets hashed and relinked to its blob --
//!   donated into the blobcas first if the blob is missing -- and each tree's recomputed hash
//!   must match its name, or the tree is quarantined.
//!
//! Every conclusion comes from bytes hashed during this run; names and link structure are
//! never trusted, only restored.  And nothing is ever deleted: quarantine is a rename into
//! `.gar/quarantine/<run>/`, out of circulation but reversible.  Removal stays gc's job.
//!
//! Repair must not run concurrently with gc, for the same reason adds must not:
//! a fresh link can race the sweep's judgement.  And quarantining a tree is disruptive
//! to anything reading that tree.  Run repair in a quiet moment.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::caswalk::for_each_shard_entry;
use crate::repo;

/// What a repair pass did: counts for reporting, and where anything quarantined went.
///
/// The relink, donation, and mode counts are repairs after which the heap is whole --
/// nothing was lost.  The quarantine counts mean corruption was found;
/// the caller decides how loudly to fail.
#[derive(Debug, Default)]
pub struct Report {
	pub blobs_verified: usize,
	pub blob_modes_normalized: usize,
	pub blobs_quarantined: usize,
	pub files_relinked: usize,
	pub blobs_donated: usize,
	pub trees_verified: usize,
	pub trees_quarantined: usize,
	/// This run's quarantine directory; None when nothing needed quarantining.
	pub quarantine_dir: Option<PathBuf>,
}

/// Verifies and repairs the whole heap.  See the module docs for the shape of the work.
pub fn repair(repo: &repo::Repo) -> io::Result<Report> {
	let mut report = Report::default();
	let mut quarantine = Quarantine {
		repo,
		run_dir: None,
	};
	// Every inode verified during the blob pass, with the (hash, exec) its blob name claims
	// and its content proved.  A treecas file sitting on one of these inodes *is* the blob:
	// correctly linked, correctly moded, and already hashed -- nothing to re-read.
	let mut verified: HashMap<u64, (gittree::Hash, bool)> = HashMap::new();

	// Blob pass.
	// Entries that don't wear a canonical blob name aren't repair's to judge, and are left alone.
	for_each_shard_entry(repo.blobcas_path(), &mut |ent| {
		if !ent.file_type()?.is_file() {
			return Ok(());
		}
		let name = ent.file_name();
		let Some(name) = name.to_str() else {
			return Ok(());
		};
		let Some((hash, exec)) = parse_blob_name(name) else {
			return Ok(());
		};
		let path = ent.path();
		let meta = ent.metadata()?;
		// Two blob names sharing an inode can't both be right.  Differing hashes on one inode
		// means at most one name matches the content (the other fails the hash check whichever
		// order they're met in).  The sneaky case is `h` and `h-x` sharing: both would hash true,
		// but no single mode satisfies both names, and normalizing one clobbers the other --
		// repair would never be idempotent.  So the second name met on any verified inode
		// is quarantined unread.
		if verified.contains_key(&meta.ino()) {
			quarantine.take("blobcas", name, &path)?;
			report.blobs_quarantined += 1;
			return Ok(());
		}
		let mut f = fs::File::open(&path)?;
		let actual = gittree::hash_of_stream(&mut f, meta.size())?;
		if actual != hash {
			quarantine.take("blobcas", name, &path)?;
			report.blobs_quarantined += 1;
			return Ok(());
		}
		let want_mode = if exec {
			repo::BLOB_MODE_EXEC
		} else {
			repo::BLOB_MODE
		};
		if meta.permissions().mode() & 0o7777 != want_mode {
			// An fchmod via the open handle, so it lands on the inode we just hashed
			// even if the name was swapped out from under us.
			f.set_permissions(fs::Permissions::from_mode(want_mode))?;
			report.blob_modes_normalized += 1;
		}
		verified.insert(meta.ino(), (hash, exec));
		report.blobs_verified += 1;
		Ok(())
	})?;

	// Tree pass.
	// Same recognition rule as gc: a tree entry is a directory wearing a canonical hash name
	// at shard depth, and anything else in there isn't repair's to judge.
	for_each_shard_entry(repo.treecas_path(), &mut |ent| {
		if !ent.file_type()?.is_dir() {
			return Ok(());
		}
		let name = ent.file_name();
		let Some(name) = name.to_str() else {
			return Ok(());
		};
		let Some(claimed) = gittree::Hash::from_hex_strict(name) else {
			return Ok(());
		};
		match walk_dir(repo, &ent.path(), &mut verified, &mut report)? {
			Some(actual) if actual == claimed => {
				report.trees_verified += 1;
			},
			// The tree's content provably isn't what its name promises.  Whether the bytes
			// drifted or the name lies is undecidable from here; what's certain is that
			// handing this directory out under this hash would be serving corruption.
			_ => {
				quarantine.take("treecas", name, &ent.path())?;
				report.trees_quarantined += 1;
			},
		}
		Ok(())
	})?;

	report.quarantine_dir = quarantine.run_dir;
	Ok(report)
}

/// Parses a blobcas file name: 64 lowercase hex characters, with an optional `-x` suffix
/// marking the executable variant.  Anything else is not a blob name.
fn parse_blob_name(name: &str) -> Option<(gittree::Hash, bool)> {
	let (hex, exec) = match name.strip_suffix("-x") {
		Some(hex) => (hex, true),
		None => (name, false),
	};
	gittree::Hash::from_hex_strict(hex).map(|h| (h, exec))
}

/// Rehashes one directory of a treecas entry, repairing file links along the way.
///
/// Returns `None` for a directory no git tree can represent -- one containing an entry
/// that is neither file, symlink, nor directory.  Such a tree recomputes to no hash at all,
/// and the caller quarantines it.
fn walk_dir(
	repo: &repo::Repo,
	dir: &Path,
	verified: &mut HashMap<u64, (gittree::Hash, bool)>,
	report: &mut Report,
) -> io::Result<Option<gittree::Hash>> {
	let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, io::Error>>()?;
	gittree::sort_dirents_gitwise(&mut entries);
	let mut tha = gittree::TreeHashAccumulator::new(entries.len());
	for ent in entries {
		let ft = ent.file_type()?;
		let file_name = ent.file_name(); // for lifetime purposes.
		let fnb = file_name.as_os_str().as_encoded_bytes();
		if ft.is_file() {
			let (hash, exec) = repair_file(repo, &ent.path(), &ent.metadata()?, verified, report)?;
			if exec {
				tha.append_executable(fnb, &hash);
			} else {
				tha.append_file(fnb, &hash);
			}
		} else if ft.is_symlink() {
			let hash = gittree::hash_of_symlink(ent.path())?;
			tha.append_symlink(fnb, &hash);
		} else if ft.is_dir() {
			match walk_dir(repo, &ent.path(), verified, report)? {
				Some(hash) => tha.append_dir(fnb, &hash),
				None => return Ok(None),
			}
		} else {
			return Ok(None);
		}
	}
	Ok(Some(tha.finish()))
}

/// Brings one treecas file back onto a blobcas inode, and returns the (hash, exec)
/// its tree entry accumulates with.
///
/// A file sitting on an inode the blob pass verified is already a correct hardlink:
/// nothing to do, nothing to read.  Any other inode gets hashed fresh.
/// If its blob is missing from the blobcas, this file donates it: the inode is chmod'd
/// to blob-normal mode and linked in, becoming the blob.  Otherwise the file is
/// atomically replaced with a link to the existing blob.
fn repair_file(
	repo: &repo::Repo,
	path: &Path,
	meta: &fs::Metadata,
	verified: &mut HashMap<u64, (gittree::Hash, bool)>,
	report: &mut Report,
) -> io::Result<(gittree::Hash, bool)> {
	if let Some((hash, exec)) = verified.get(&meta.ino()) {
		return Ok((hash.clone(), *exec));
	}
	// Hash and exec bit come from this one fd, as in add: they describe the same inode
	// even if the path is swapped out mid-walk.
	let mut f = fs::File::open(path)?;
	let meta = f.metadata()?;
	let exec = gittree::mode_is_executable(meta.permissions().mode());
	let hash = gittree::hash_of_stream(&mut f, meta.size())?;

	let blob_path = repo.blob_path(&hash, exec);
	if blob_path.is_file() {
		// Relink: hardlink the blob to a temp name in its shard dir, then rename over the file.
		// The rename is atomic, so an interruption leaves the old file or the new link, never absence.
		// The temp lives in the shard dir, not the tree: a crash may abandon it, and dot-names
		// in shard dirs are temps by convention (walks skip them), while inside a tree
		// it would read as content and poison the tree's hash.
		let temp = blob_path.with_file_name(temp_name(&blob_path));
		match fs::hard_link(&blob_path, &temp) {
			// A leftover temp from a crashed run under a recycled pid is, by this name
			// construction, already a link to this same blob: renaming it into place
			// is exactly what was underway.
			Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
			r => r?,
		}
		fs::rename(&temp, path)?;
		report.files_relinked += 1;
	} else {
		// Donate.  Mode goes first: a blobcas inode is never touched again after the name exists.
		let mode = if exec {
			repo::BLOB_MODE_EXEC
		} else {
			repo::BLOB_MODE
		};
		f.set_permissions(fs::Permissions::from_mode(mode))?;
		repo.create_blob_shard(&hash)?;
		match fs::hard_link(path, &blob_path) {
			// Someone else (a racing add) published this blob between our check and our link.
			// The name is the consensus; this file keeping its own content-true inode
			// costs a little dedup, never correctness.
			Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
			r => r?,
		}
		report.blobs_donated += 1;
		verified.insert(meta.ino(), (hash.clone(), exec));
	}
	Ok((hash, exec))
}

/// The temp name a relink stages under: dot-prefixed (so shard walks skip it),
/// pid-disambiguated (like gc's removal temps), and carrying the blob name it links.
fn temp_name(blob_path: &Path) -> String {
	let blob_name = blob_path
		.file_name()
		.and_then(|n| n.to_str())
		.expect("a blob path ends in a hex name");
	format!(".repair-{}-{}", std::process::id(), blob_name)
}

/// This run's quarantine directory, made lazily on first use:
/// a repair that finds nothing wrong leaves no trace.
struct Quarantine<'a> {
	repo: &'a repo::Repo,
	run_dir: Option<PathBuf>,
}

impl Quarantine<'_> {
	/// Moves one CAS entry out of circulation, preserving its sharded path under
	/// `.gar/quarantine/<unix-seconds>-<pid>/{blobcas,treecas}/`.
	/// A rename, not a copy: same filesystem, instant, and it keeps the inode family intact
	/// for anyone doing forensics later.
	///
	/// The run dir name is unix seconds plus pid: enough to keep runs apart
	/// (including a crashed run retried in the same second) without reaching for a date library.
	fn take(&mut self, kind: &str, name: &str, src: &Path) -> io::Result<()> {
		let run_dir = match &self.run_dir {
			Some(d) => d,
			None => {
				let secs = std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.expect("the clock reads after 1970")
					.as_secs();
				let d =
					self.repo
						.quarantine_path()
						.join(format!("{}-{}", secs, std::process::id()));
				fs::create_dir_all(&d)?;
				self.run_dir.insert(d)
			},
		};
		let shard = run_dir.join(kind).join(&name[0..2]).join(&name[2..4]);
		fs::create_dir_all(&shard)?;
		fs::rename(src, shard.join(name))
	}
}
