use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub struct Repo {
    /// Path as originally specified.
    /// Includes ".gar" (unless this is a bare repo).
    /// Usually this used in messages to the user.
    path: PathBuf,

    /// Absolute and canonicalized version of the path.
    /// This is used to do checks like if other files we might walk are within the repo root.
    /// (Yes, it would be better to do that with handle-based APIs, but alas, those are in short supply.)
    path_abs: PathBuf,

    // All of the paths below are suffixes of 'self.path_abs', but used so frequently we create them once.
    blobcas_path: PathBuf,
    treecas_path: PathBuf,
    treeidx_path: PathBuf,
}

impl Repo {
    pub fn new(root_path: impl AsRef<Path>) -> Result<Self, io::Error> {
        Self::new_bare(root_path.as_ref().join(".gar"))
    }
    pub fn new_bare(root_path: impl AsRef<Path>) -> Result<Self, io::Error> {
        let path = root_path.as_ref().to_owned();
        let path_abs = fs::canonicalize(&path)?;
        Ok(Repo {
            path,
            blobcas_path: (&path_abs).join("blobcas"),
            treecas_path: (&path_abs).join("treecas"),
            treeidx_path: (&path_abs).join("treeidx"),
            path_abs,
        })
    }

    pub fn create_dir_all(&self) -> io::Result<()> {
        fs::create_dir_all(self.repo_path())?;
        fs::create_dir_all(self.blobcas_path())?;
        fs::create_dir_all(self.treecas_path())?;
        fs::create_dir_all(self.treeidx_path())?;
        Ok(())
    }

    pub fn repo_path(&self) -> &Path {
        &self.path
    }
    pub fn repo_path_abs(&self) -> &Path {
        &self.path_abs
    }
    pub fn blobcas_path(&self) -> &Path {
        &self.blobcas_path
    }
    pub fn treecas_path(&self) -> &Path {
        &self.treecas_path
    }
    pub fn treeidx_path(&self) -> &Path {
        &self.treeidx_path
    }
}

pub fn find_repo() -> Result<Option<Repo>, io::Error> {
    return find_repo_from(std::env::current_dir()?);
}

pub fn find_repo_from(p: impl AsRef<Path>) -> Result<Option<Repo>, io::Error> {
    let path = p.as_ref();
    if path.join(".gar").exists() {
        return Ok(Some(Repo::new(path)?));
    }
    match path.parent() {
        Some(p) => return find_repo_from(p),
        None => return Ok(None),
    }
}
