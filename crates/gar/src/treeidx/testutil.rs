//! Fixtures and helpers shared by the treeidx tests.

use std::fs;
use std::path::PathBuf;

/// The root hash of gittree's `alpha` fixture tree, as gittree's own tests pin it.
pub(super) const ALPHA_ROOT: &str =
	"9024a7f8afa43db06ff2b50d9ac9c21b791bee49d8092d3f14f1e433bfd927fa";

fn manifest_dir() -> PathBuf {
	PathBuf::from(
		std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR for tests"),
	)
}

/// gittree's fixture tree: files, nested dirs, and a symlink.
pub(super) fn alpha_tree() -> PathBuf {
	manifest_dir().join("../gittree/fixtures/alpha")
}

/// The expected treeidx bytes for `alpha_tree`, written by hand from the hashes gittree's tests pin.
pub(super) fn alpha_golden() -> Vec<u8> {
	fs::read(manifest_dir().join("fixtures/treeidx/alpha.treeidx"))
		.expect("the golden fixture is readable")
}

pub(super) fn hash(hex: &str) -> gittree::Hash {
	gittree::Hash::from_hex_strict(hex).expect("test hashes are well-formed")
}

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
pub(super) fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}
