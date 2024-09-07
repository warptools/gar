use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use crate::gittree;
use crate::repo;

pub enum FaithMode {
    Copy,
    LinkOriginals,
}

pub fn add(repo: &repo::Repo, path: &Path, faithmode: FaithMode) -> io::Result<gittree::Hash> {
    // Make the tempdir in the treecas that we'll fill.
    // (Even if the entire tree turns out to be familar... that's fine, but there's no real shortcut to noticing it,
    // and in that filling this tree with newly writen hardlinks isn't much more expensive than just doing the walk;
    // whereas doing two walks after checking for newness and then actual doing would be quite the waste.)
    let td = tempdir::TempDir::new_in(repo.treecas_path(), ".wiptree-")?;

    // TODO: have not handled the case where a single file is given as target.  That doesn't really produce a treecas, by most definitions.
    // I'm not exactly sure what the correct UX is for that.

    // Walk the filesystem.  Depth first.
    add_recurse(
        path,
        repo.blobcas_path(),
        td.path(),
        Path::new(""),
        &fs::metadata(path)?,
        faithmode,
    )
}

/// Walk the filesystem.  Depth first.
/// Hardlink (or copy, depending on faithmode) stuff into the repo's blobcas,
/// and accumulate it into tree_path (which is in treecas).
/// Return the treehash (or blobhash) at ever step.
///
/// This code path does not apply for single files.
fn add_recurse(
    scan_root: &Path,
    blobcas_root: &Path,
    wiptree_root: &Path,
    path: &Path,
    path_meta: &fs::Metadata,
    faithmode: FaithMode,
) -> io::Result<gittree::Hash> {
    let path_ft = path_meta.file_type();
    if path_ft.is_file() {
        // Files:

        // Blobhash each file.  Gotta know where to put it.
        let hash = gittree::hash_of_stream(&mut fs::File::open(path)?, path_meta.size())?;

        // First: Populate the blobcas, either by copy or by hardlink.
        // BRANCH: are we in paranoia mode, or are we hardlinking orignals and trusting in a lack of mutation?
        // Note that in the copy mode, we use `io::copy` rather than `fs::copy`, because the latter puts work into copying permissions, attribs, etc, and we have no need for that.
        let blobcas_path = blobcas_root.join(hash.as_hex());
        match faithmode {
            FaithMode::Copy => {
                todo!("i sure wish our hashing and copying could work on one read pass")
                // io::copy(reader, writer)
            }
            FaithMode::LinkOriginals => fs::hard_link(scan_root.join(path), blobcas_path)?,
            // REVIEW: what do if permissions aren't normal?  Copy instead?  Add a mode for halt-if-not-ezlinkable?
        }

        // Second: hardlink a new entry in the treecas to the blobcas.
        fs::hard_link(scan_root.join(path), wiptree_root.join(path))?;

        // And return the hash so dir treehashing can accumulate.
        Ok(hash)
    } else if path_ft.is_symlink() {
        // Symlinks:

        // There's no point in blobcas'ing these, but we still do need their git hash to construct tree IDs, so do so.
        // TODO make `gittree::hash_of_symlink` return the body too so we can DRY this better
        let target = fs::read_link(path)?;
        let mut foo = target.as_os_str().as_encoded_bytes();
        let size = foo.len().try_into().expect("int size nonsense");
        let hash = gittree::hash_of_stream(&mut foo, size)?;

        // Make a new symlink in the treecas output dir.
        std::os::unix::fs::symlink(target, wiptree_root.join(path))?;

        // And return the hash so dir treehashing can accumulate.
        Ok(hash)
    } else if path_ft.is_dir() {
        // Dirs:
        // We go ahead and make the path of the same name in the temp new treecas.
        // Then recurse.  Most of the work is in filling this dir up; then, just a little hashing work at the end.
        for entry in fs::read_dir(scan_root.join(path))? {
            let entry = entry?;
            let ft = entry.file_type()?;
        }
        Ok(todo!(
            "extract a dir buffer builder in gittree module and use it hereabouts"
        ))
    } else {
        panic!("unknown file type")
    }
}
