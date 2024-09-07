use std::path::Path;
use std::path::PathBuf;

pub struct Repo {
    path: PathBuf,

    // All of the paths below are created once and references are valid as long as the Repo object exists.
    blobcas_path: Box<Path>,
    treecas_path: Box<Path>,
    treeidx_path: Box<Path>,
}

impl Repo {
    pub fn treecas_path(&self) -> &Path {
        &self.treecas_path
    }
    pub fn blobcas_path(&self) -> &Path {
        &self.blobcas_path
    }
}
