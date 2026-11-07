use std::fs;
use std::io;
use std::io::Seek as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::blob_landing;
use crate::index_landing;
use crate::repo;
use crate::treeidx;
use gittree;

/// Stores the tree at `path` in the heap, and returns its treehash.
///
/// The tree's index is written too; `add_with` can switch that off.
pub fn add(
	repo: &repo::Repo,
	path: impl AsRef<Path>,
	faithmode: FaithMode,
) -> Result<gittree::Hash, io::Error> {
	add_with(repo, path, faithmode, Indexing::Write)
}

/// `add`, with the choice of whether to write the tree's index.
pub fn add_with(
	repo: &repo::Repo,
	path: impl AsRef<Path>,
	faithmode: FaithMode,
	indexing: Indexing,
) -> Result<gittree::Hash, io::Error> {
	// Make the tempdir in the treecas root: we'll fill into this,
	// then move the tempdir into it's CAS-named place at the very end.
	//
	// (n.b., Even if the entire tree we're adding turns out to be familiar... there's no real shortcut to noticing it.
	// We'll notice only at the end, when we're about ready to commit to CAS.
	// In Copy mode, this may be unfortunate, because we've done all this write IO already and now have to drop it;
	// but in either case, we had to do all the read IO, and there's simply no way to avoid _that_.)
	let td = tempdir::TempDir::new_in(repo.treecas_path(), ".wiptree-")?;

	// Copy mode stages each file here before publishing it into the blobcas.
	// It sits inside the repo dir so the publishing hardlink never crosses a filesystem.
	// Cleaned up on drop, same as the wiptree.
	let staging = tempdir::TempDir::new_in(repo.repo_path(), ".staging-")?;

	// TODO: have not handled the case where a single file is given as target.  That doesn't really produce a treecas, by most definitions.
	// I'm not exactly sure what the correct UX is for that.
	// Gar may just end up forbidding this because I don't know what else it should do.

	let mut w = AddWork {
		repo,
		repo_ino: fs::metadata(repo.repo_path())?.ino(),
		scan_root: path.as_ref(),
		wiptree_root: td.path(),
		staging_root: staging.path(),
		faithmode,
		index: match indexing {
			Indexing::Write => Some(Vec::new()),
			Indexing::Skip => None,
		},
		// Overwritten in every directory's slot before the walk returns.
		placeholder: gittree::Hash::from_hex([b'0'; 64]).expect("64 zeros are valid hex"),
	};

	// Walk the filesystem.
	// (n.b., individual things start entering the blobcas immediately;
	// even though we have one big commit at the end for the whole treecas,
	// removing things from the blobcas as soon as this walk begins would require a GC.)
	let hash = w.add_recurse(Path::new(""), &fs::metadata(w.scan_root)?)?;

	// The index is checked before the commit, so a tree that can't be indexed is never stored.
	// It's written only after the commit, so no index ever names a tree the heap lacks.
	let index_bytes = w.index.take().map(serialize_index).transpose()?;

	// The final commit: move the whole wiptree into CAS place.
	// Shard dirs are created only after a `NotFound` says they're missing, same as blob placement.
	//
	// A failed rename is tolerated if a directory holds the destination: in a CAS, losing
	// the rename race to another add manifesting the same tree is success.  The check is on the
	// destination afterward, not the error kind, because the kinds don't carve this cleanly:
	// POSIX allows either `EEXIST` or `ENOTEMPTY` for a nonempty directory target, and a rename
	// onto an existing *empty* directory (the empty tree) succeeds outright.  "Is a directory
	// there now?" is the actual question.  A non-directory squatting at the name is not that
	// tree -- returning success would hand back a hash for a heap that lacks it -- so that,
	// and every other failure, stays an error.
	let dest_path: PathBuf = repo.tree_path(&hash);
	let mut treecas_result = fs::rename(&td, &dest_path);
	if let Err(e) = &treecas_result {
		if e.kind() == io::ErrorKind::NotFound {
			repo.create_tree_shard(&hash)?;
			treecas_result = fs::rename(&td, &dest_path);
		}
	}
	if let Err(e) = treecas_result {
		if !dest_path.is_dir() {
			return Err(e);
		}
	}

	// The tree is in stock either way now, so the index goes in on both of those paths.
	if let Some(bytes) = index_bytes {
		index_landing::land_index(repo, &hash, &bytes)?;
	}

	Ok(hash)
}

/// Verifies and serializes the index entries add's walk recorded.
///
/// The walk and `verify` each compute every tree hash, so a refusal means they disagree (a bug),
/// or that a path is too long for the format's length field.
/// Either way the add fails rather than storing a tree without its index.
fn serialize_index(entries: Vec<treeidx::Entry>) -> io::Result<Vec<u8>> {
	match treeidx::verify(entries) {
		Ok(idx) => Ok(treeidx::serialize(&idx)),
		Err(refusal) => Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!("cannot index the tree being added: {refusal}"),
		)),
	}
}

/// Whether an add writes a treeidx file for the tree it stores.
#[derive(Clone, Copy, Debug)]
pub enum Indexing {
	/// Write the index, built from the add's own walk (no content is hashed twice).
	/// This is what `add` does.
	Write,
	/// Store the tree with no index.
	/// One can still be generated from the stored tree later, at the cost of re-hashing it.
	Skip,
}

#[derive(Clone, Copy, Debug)]
pub enum FaithMode {
	/// Copy files into the blobstore while adding to gar.
	///
	/// This is a wise choice if you don't know if the originals will be mutated
	/// again in the future (even after the add process returns!).
	Copy,

	/// Hardlink originals into the blobstore while adding to gar.
	///
	/// Only do this when you're very sure the original files won't be mutated ever again.
	/// A mutation to the original will mutate all hardlinked files (they're not distinct files!),
	/// and that would result in a corruption of the gar blobcas and thereafter undefined (and bad) behavior.
	LinkOriginals,

	/// Move originals into the blobstore while adding to gar.
	/// This results in the files given to the "add" command being removed from their original locations.
	///
	/// This can be a suitable option if you made a file set entirely for the purpose of adding to gar.
	/// (A similar outcome is possible with `LinkOriginals` mode followed by a recursive rm,
	/// but `Move` mode will save on that handful of rm syscalls.)
	Move,
	// On second thought, I don't really know why I'd want this "move" mode, because the performance gain is irrelevant,
	// and the situation you'd end up in if there's an interruption of some kind would be quite unpleasant unless the
	// subject directory structures were totally disposable.

	// Some filesystems support a concept of "reflinking", which is the COW we've always wanted
	// and *does* create new files with fresh attributes and no mutation craziness.
	// Unfortunately, this is far from a universally available feature, and we're concerned with that.

	// Possible future work: It might be useful to have some more mode variants to describe situations like
	// "hardlink if you can; but fall back to copy without erroring if it would be cross-device".
}

/// Bundles up all the parameters we'd pass down in recursion.
struct AddWork<'a> {
	repo: &'a repo::Repo,
	repo_ino: u64,
	scan_root: &'a Path,
	wiptree_root: &'a Path,
	staging_root: &'a Path,
	faithmode: FaithMode,
	/// The tree's index entries so far, in preorder; `None` when not indexing.
	index: Option<Vec<treeidx::Entry>>,
	placeholder: gittree::Hash,
}

impl AddWork<'_> {
	/// Walk the filesystem.  Depth first.
	/// Hardlink (or move, or copy, depending on faithmode) stuff into the repo's blobcas,
	/// and then hardlink *that* it into wiptree.
	/// Return the treehash (or blobhash) at every step
	fn add_recurse(&mut self, path: &Path, path_meta: &fs::Metadata) -> io::Result<gittree::Hash> {
		// TODO: this whole function is somewhat silly.  It is only used at the root, and even there does not do anything special.  Remove it.
		let path_ft = path_meta.file_type();
		if path_ft.is_file() {
			self.add_recurse_file(path).map(|(hash, _)| hash)
		} else if path_ft.is_symlink() {
			self.add_recurse_symlink(path)
		} else if path_ft.is_dir() {
			self.add_recurse_dir(path)
		} else {
			panic!("unknown file type")
		}
	}

	/// Records an index entry for `path`, if this add is writing an index, and returns its position.
	///
	/// Every entry is recorded by the step that put it in the wiptree, so the index mirrors what landed.
	fn record(&mut self, path: &Path, kind: treeidx::Kind) -> Option<usize> {
		let index = self.index.as_mut()?;
		index.push(treeidx::Entry {
			path: path.as_os_str().as_encoded_bytes().to_vec(),
			kind,
		});
		Some(index.len() - 1)
	}

	/// Copy-mode blob intake: copies the open source into a private staging file,
	/// normalizes its permissions, hashes the copy, and publishes it into the blobcas.
	///
	/// Copy first, hash the copy afterward -- don't tee a hash out of the copy, and don't hash the source:
	/// the plain copy takes kernel fast paths that make it plus a re-read cheaper than streaming
	/// through userland (see `experiments/benches/direct_copy.rs` for evidence),
	/// and hashing the bytes we actually publish means a source file mutated mid-add
	/// still can't put content under a CAS name that doesn't describe it.
	///
	/// Returns the hash and size of the copy: those describe the blob that landed,
	/// even if the source changed underfoot.
	fn copy_blob_in(
		&self,
		source: &mut fs::File,
		executable: bool,
	) -> io::Result<(gittree::Hash, u64)> {
		let staging_path = self.staging_root.join("staged");
		let mut staged = fs::OpenOptions::new()
			.read(true)
			.write(true)
			.create_new(true)
			.open(&staging_path)?;
		io::copy(source, &mut staged)?;
		// This is an fchmod, so the published mode is exact, umask notwithstanding.
		// It must happen before the publishing link below: a blobcas inode is never touched again.
		let mode = if executable {
			repo::BLOB_MODE_EXEC
		} else {
			repo::BLOB_MODE
		};
		staged.set_permissions(fs::Permissions::from_mode(mode))?;
		let size = staged.metadata()?.size();
		staged.seek(io::SeekFrom::Start(0))?;
		let hash = gittree::hash_of_stream(&mut staged, size)?;
		blob_landing::place_blob(self.repo, &staging_path, &hash, executable)?;
		fs::remove_file(&staging_path)?;
		Ok((hash, size))
	}

	/// Returns the blob hash, and whether the file was stored as executable (which the tree entry mode must match).
	fn add_recurse_file(&mut self, path: &Path) -> io::Result<(gittree::Hash, bool)> {
		// Files:
		// - First get it in the blobcas.
		// - Then hardlink it into the treecas.
		//
		// The treecas is *always* a hardlink to the blobcas,
		// and also we'll outright error if that gives a cross-device link, because what are you even doing.
		//
		// Getting it into the blobcas... that can happen a couple different ways; that's what the FaithMode param is about.

		// TODO: some design choices still needed about metadata in the case of hardlinking.
		// This is a little tricky to figure out, because we want:
		//   - to support hardlinking the original, if requested
		//   - but *maybe* we don't want to do that if the permissions differ from our normalized permissions
		//   - or *maybe* we'll respond to non-normal permissions by normalizing them, if requested
		//   - and then (orthagonal, except it might skip some of the other choices) there's the *maybe* fallback to copy if hardlink would be a cross-device link.

		// Open the source exactly once: the exec bit (and, in link mode, size and content)
		// comes from this one fd, so it describes the same inode
		// even if the path is chmod'ed or renamed over mid-walk.
		let mut file = fs::File::open(self.scan_root.join(path))?;
		let file_meta = file.metadata()?;
		let executable = gittree::mode_is_executable(file_meta.permissions().mode());

		// First: populate the blobcas; how, depends on the faith mode.
		// There is one piece of data we have to represent in the blob name beyond the hash itself:
		// because git stores the executable bit in the tree, rather than the blob header itself,
		// we have to store the executable bit as a suffix of the blobhash.
		// (We can't just add executable bit back onto things in the treecas, because that's...
		// not how hardlinks work, unfortunately.  Oh how I wish it was!  But, nope.)
		let (hash, size) = match self.faithmode {
			FaithMode::Copy => self.copy_blob_in(&mut file, executable)?,

			// In LinkOriginals mode, hardlinks are made into the blobcas straight to the original file.
			// However, this is dangerous if the original file is ever modified again: hardlinks effectively transmit mutations.
			FaithMode::LinkOriginals => {
				let hash = gittree::hash_of_stream(&mut file, file_meta.size())?;
				blob_landing::place_blob(self.repo, &self.scan_root.join(path), &hash, executable)?;
				(hash, file_meta.size())
			},
			FaithMode::Move => todo!(),
			// REVIEW: what do if permissions aren't normal in LinkOriginals mode?  Copy instead?  Add a mode for halt-if-not-ezlinkable?
		};

		// Second: hardlink a new entry in the treecas to the blobcas.
		fs::hard_link(
			self.repo.blob_path(&hash, executable),
			self.wiptree_root.join(path),
		)?;
		self.record(
			path,
			treeidx::Kind::File {
				executable,
				size,
				hash: hash.clone(),
			},
		);

		// And return the hash so dir treehashing can accumulate.
		Ok((hash, executable))
	}

	fn add_recurse_symlink(&mut self, path: &Path) -> io::Result<gittree::Hash> {
		// There's no point in blobcas'ing these, but we still do need their git hash to construct tree IDs, so do so.
		// TODO make `gittree::hash_of_symlink` return the body too so we can DRY this better
		let target = fs::read_link(self.scan_root.join(path))?;
		let mut foo = target.as_os_str().as_encoded_bytes();
		let size = foo.len().try_into().expect("int size nonsense");
		let hash = gittree::hash_of_stream(&mut foo, size)?;

		// Make a new symlink in the treecas output dir.
		std::os::unix::fs::symlink(&target, self.wiptree_root.join(path))?;
		self.record(
			path,
			treeidx::Kind::Symlink {
				target: target.into_os_string().into_encoded_bytes(),
			},
		);

		// And return the hash so dir treehashing can accumulate.
		Ok(hash)
	}

	fn add_recurse_dir(&mut self, path: &Path) -> io::Result<gittree::Hash> {
		// The dir's entry precedes its children's in the index, but its hash is only known after them,
		// so it's recorded with a placeholder now and filled in at the end.
		let slot = self.record(
			path,
			treeidx::Kind::Dir {
				hash: self.placeholder.clone(),
			},
		);

		// Begin to walk.
		// Sort all entries first; we need to form the tree data this way.
		let scan_path = self.scan_root.join(path);
		let mut entries = fs::read_dir(scan_path)?.collect::<Result<Vec<_>, io::Error>>()?;
		gittree::sort_dirents_gitwise(&mut entries);

		// Accumulation begins.
		let mut tha = gittree::TreeHashAccumulator::new(entries.len());

		for ent in entries {
			let ft = ent.file_type()?;
			let file_name = ent.file_name(); // for lifetime purposes.
			let fnb = file_name.as_os_str().as_encoded_bytes();

			if ft.is_file() {
				let (hash, executable) = self.add_recurse_file(&path.join(&file_name))?;
				if executable {
					// TODO: we should probably normalize that if any of those bits are set, all of them are set.
					// Among other defensible normalizations that would certainly occur during a copy.
					tha.append_executable(fnb, &hash);
				} else {
					tha.append_file(fnb, &hash);
				}
			} else if ft.is_symlink() {
				let hash = self.add_recurse_symlink(&path.join(&file_name))?;
				tha.append_symlink(fnb, &hash);
			} else if ft.is_dir() {
				// Special case: if we're encounter the repo itself: do not add that!
				// (This is not super uncommon: "gar add ." is generally expected to DTRT.)
				if ent.metadata()?.ino() == self.repo_ino {
					continue;
				}

				// Go ahead and make the path of the same name in the temp new treecas.
				// (This happens here because at the very root, we don't need to do it, because we started with a tempdir there already.)
				let wip_path = self.wiptree_root.join(path.join(&file_name));
				std::fs::create_dir(wip_path)?;
				// Recurse.
				let hash = self.add_recurse_dir(&path.join(&file_name))?;
				tha.append_dir(fnb, &hash);
			} else {
				panic!("unknown file type")
			}
		}
		let hash = tha.finish();
		if let (Some(index), Some(slot)) = (&mut self.index, slot) {
			index[slot].kind = treeidx::Kind::Dir {
				hash: hash.clone(),
			};
		}
		Ok(hash)
	}
}
