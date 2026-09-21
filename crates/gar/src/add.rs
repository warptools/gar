use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::repo;
use gittree;

pub fn add(
    repo: &repo::Repo,
    path: impl AsRef<Path>,
    faithmode: FaithMode,
) -> Result<gittree::Hash, io::Error> {
    // Make the tempdir in the treecas root: we'll fill into this,
    // then move the tempdir into it's CAS-named place at the very end.
    //
    // (n.b., Even if the entire tree we're adding turns out to be familiar... there's no real shortcut to noticing it.
    // We'll notice only at the end, when we're about ready to commit to CAS.
    // In Copy mode, this may be unfortunate, because we've done all this write IO already and now have to drop it;
    // but in either case, we had to do all the read IO, and there's simply no way to avoid _that_.)
    let td = tempdir::TempDir::new_in(repo.treecas_path(), ".wiptree-")?;

    // TODO: have not handled the case where a single file is given as target.  That doesn't really produce a treecas, by most definitions.
    // I'm not exactly sure what the correct UX is for that.
    // Gar may just end up forbidding this because I don't know what else it should do.

    let w = AddWork {
        repo,
        repo_ino: fs::metadata(repo.repo_path())?.ino(),
        scan_root: path.as_ref(),
        wiptree_root: td.path(),
        faithmode,
    };

    // Walk the filesystem.
    // (n.b., individual things start entering the blobcas immediately;
    // even though we have one big commit at the end for the whole treecas,
    // removing things from the blobcas as soon as this walk begins would require a GC.)
    let hash = w.add_recurse(Path::new(""), &fs::metadata(w.scan_root)?)?;

    // The final commit: move the whole wiptree into CAS place.
    let dest_path: PathBuf = repo.treecas_path().join(hash.as_hex());
    let treecas_result = fs::rename(&td, &dest_path);

    // The error handling for this last step, however, is... wild.
    //
    // Goal: if the rename failed, because the target exists, that's *fine*;
    // because we're a CAS system, that just means someone else raced us
    // to manifesting that result already, and that's _fine_.
    //
    // Actual: uh, well.  Oof.
    // As far as I can tell, telling when a directory rename fails because target exists
    // is literally programmatically indistinguishable in the rust stdlib as of today on stable.
    // So... what we're going to do is, for _any_ error, just check if the target path exists,
    // and if it does, we'll discard the error (regardless of what it was).
    //
    // (r u serious??  Yeah, yeah I am.  fs::rename doesn't return an
    // `io::ErrorKind::AlreadyExists`, which is what you might expect.
    // Instead, it returns `io::ErrorKind::DirectoryNotEmpty`.  Which...
    // is illegal to talk about, because it's behind the 'io_error_more' unstable feature
    // (see <https://github.com/rust-lang/rust/issues/86442>).
    // I don't see any alternative route to the data, except for maybe dumping the debug
    // format into a buffer and matching on strings, which is heinous.
    // I'm also not willing to start using an unstable compiler for just this.
    // So.
    //
    // Fortunately, checking if the target directory already exists *after* our move attempt
    // is fine: although it _smells_ like a TOCTOU, we're in a situation where it's fine.
    // We don't care about the ability to distinguish if this process succeeded; so long as
    // *someone* succeeded in making that data manifest, we're happy.  And because a CAS
    // system is append-only (outside of GC), there's no relevant race conditions in sight.
    //
    // The other way to attack this might be to switch to the 'rustix' crate
    // and attempt to leave rust's stdlib fs API behind entirely.  Tempting.
    // But a bridge too far for me today.
    match treecas_result {
        Ok(_) => {} // cool
        Err(e) => {
            // Neither of these two conditions is valid:
            //   - `e.kind() == io::ErrorKind::AlreadyExists`
            //      --> is what I expected, but is not actually what the fs package produces.
            //   - `e.kind() == io::ErrorKind::DirectoryNotEmpty`
            //      --> is what we really see, except its hidden behind `#![feature(io_error_more)]`.
            //
            // So... Check the actual outcomes instead.
            // Return the error from renaming only if the destination doesn't exist.
            if !dest_path.exists() {
                return Err(e);
            }
        }
    };

    Ok(hash)
}

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
    faithmode: FaithMode,
}

impl AddWork<'_> {
    /// Walk the filesystem.  Depth first.
    /// Hardlink (or move, or copy, depending on faithmode) stuff into the repo's blobcas,
    /// and then hardlink *that* it into wiptree.
    /// Return the treehash (or blobhash) at every step
    fn add_recurse(&self, path: &Path, path_meta: &fs::Metadata) -> io::Result<gittree::Hash> {
        // TODO: this whole function is somewhat silly.  It is only used at the root, and even there does not do anything special.  Remove it.
        let path_ft = path_meta.file_type();
        if path_ft.is_file() {
            self.add_recurse_file(path, path_meta)
        } else if path_ft.is_symlink() {
            self.add_recurse_symlink(path)
        } else if path_ft.is_dir() {
            self.add_recurse_dir(path)
        } else {
            panic!("unknown file type")
        }
    }

    fn add_recurse_file(&self, path: &Path, path_meta: &fs::Metadata) -> io::Result<gittree::Hash> {
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

        // Blobhash each file.  Gotta know where to put it.
        // TODO: if we have to copy, would rather hold off on this so we don't read twice.
        // // ^ ok this turns out untrue; attempts to tee in memory have been poor because smarter kernel calls that just do "copy" are faster.
        // //     so instead what we should really think about here is what's safest in the face of concurrent activity (as much as possible, and from the pov of the cas).
        let hash = gittree::hash_of_stream(
            &mut fs::File::open(self.scan_root.join(path))?,
            path_meta.size(),
        )?;

        // First: Populate the blobcas, either by copy or by hardlink.
        // There is one piece of data we have to represent in the blob name beyond the hash itself:
        // because git stores the executable bit in the tree, rather than the blob header itself,
        // we have to store the executable bit as a suffix of the blobhash.
        // (We can't just add executable bit back onto things in the treecas, because that's...
        // not how hardlinks work, unfortunately.  Oh how I wish it was!  But, nope.)
        let attrib_suffix = if gittree::mode_is_executable(path_meta.permissions().mode()) {
            "-x"
        } else {
            ""
        };
        let blobcas_path = self.repo.blobcas_path().join(hash.as_hex() + attrib_suffix);
        // BRANCH: are we in paranoia mode, or are we hardlinking orignals and trusting in a lack of mutation?
        let blobcas_result: io::Result<_> = match self.faithmode {
            FaithMode::Copy => {
                // Unimplemented.  Notes toward it, because this design thinking is easy to lose:
                //
                // Do: Copy first, and read a second time to hash.
                // Don't: try to tee the hash out of the copy.
                // (This might not be intuitive, because it seems like its doing reads twice, but:
                // Copying can take kernel fastpaths -- and both `fs::copy` and `io::copy` do! --
                // and when those engage, a subsequent "re"-read is much cheaper
                // than a read-and-write loop that streams through userland.
                // See `benches/direct_copy.rs` and its adjacent notes for evidence.)
                // Also don't: hash, and then copy.  Or at least put that behind a flag that carries cautionary documentation.
                // (While that would sometimes possibly be a bit faster,
                // no amount of speed savings are worth the downsides of the CAS geting corrupted
                // if there's a concurrent modification of a file.
                // There are cases where we can still use this -- container engines, for example --
                // but those situations are almost definitionally automated and can handle having a flag.)
                //
                // So: copy to a temp name in the blobcas first; then hash; then rename into place.
                //
                // Now, about atomicity.  There's another squick coming up here.
                // Usually, in a content-addressed storage system done on a posix filesystem,
                // you just do a rename into place unconditionally, and that's that, and it's atomic: yay, easy, and correct.
                // Here where we're trying to use hardlinks to keep things deduplicating between the blobcas and the treecas?
                // More complicated.
                //
                // When we're about to move a file to its CAS name, we have to check if that name already exists.
                // Is this ia TOCTOU vs simply doing the atomic rename?
                // On the surface, yeah, sorta; but, bear with me.
                // While a rename *is* atomic, but it leaves the target path with *the inode of the source*.
                // In our context, that means *any treecas hardlinks to the previous file would be in a different family
                // than any other new treecas links generated from this file*.
                // (And if you apply that logic over time, it means... yeah, every treecas would get linked to a new blobcas file
                // every time there's one to share... so... no one would ever... ahem... share.)
                // So check for an existing entry first, and discard the temp file rather than renaming if one is there.
                // In the end, we still do a rename, *so there's no correctness risk* from the TOCTOU race;
                // the only downside of of a race is ~one duplicated blob (a treecas file that becomes separated when the blobcas is overwritten).
                // (Something that could also be recovered later in some kind of housekeeping operation, even.)
                //
                // On the code for making the temp file: the `tempfile` crate was considered and set aside.
                // It carries a lot of os-conditional code for a job we don't need done especially well;
                // it creates files 0o600 when we want the repo's normalized permissions;
                // and its `O_TMPFILE` path is only available on some filesystems anyway.
                // A probing loop is needed for the hardlink path regardless, so we're just going to own this.
                todo!("copy mode")
            }

            // In LinkOriginals mode, hardlinks are made into the blobcas straight to the original file.
            // `fs::hard_link` fails with `AlreadyExists` rather than replacing an existing target,
            // which is exactly the non-clobbering behavior wanted here (see the inode note in the Copy arm).
            // However, this is dangerous if the original file is ever modified again: hardlinks effectively transmit mutations.
            FaithMode::LinkOriginals => fs::hard_link(self.scan_root.join(path), &blobcas_path),
            FaithMode::Move => todo!(),
            // REVIEW: what do if permissions aren't normal?  Copy instead?  Add a mode for halt-if-not-ezlinkable?
        };
        match blobcas_result {
            Ok(_) => {} // cool
            Err(e) => {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    // cool
                } else {
                    return Err(e);
                }
            }
        };

        // Second: hardlink a new entry in the treecas to the blobcas.
        fs::hard_link(&blobcas_path, self.wiptree_root.join(path))?;

        // And return the hash so dir treehashing can accumulate.
        Ok(hash)
    }

    fn add_recurse_symlink(&self, path: &Path) -> io::Result<gittree::Hash> {
        // There's no point in blobcas'ing these, but we still do need their git hash to construct tree IDs, so do so.
        // TODO make `gittree::hash_of_symlink` return the body too so we can DRY this better
        let target = fs::read_link(self.scan_root.join(path))?;
        let mut foo = target.as_os_str().as_encoded_bytes();
        let size = foo.len().try_into().expect("int size nonsense");
        let hash = gittree::hash_of_stream(&mut foo, size)?;

        // Make a new symlink in the treecas output dir.
        std::os::unix::fs::symlink(target, self.wiptree_root.join(path))?;

        // And return the hash so dir treehashing can accumulate.
        Ok(hash)
    }

    fn add_recurse_dir(&self, path: &Path) -> io::Result<gittree::Hash> {
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
                let hash = self.add_recurse_file(&path.join(&file_name), &ent.metadata()?)?;
                if gittree::mode_is_executable(ent.metadata()?.permissions().mode()) {
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
        return Ok(tha.finish());
    }
}
