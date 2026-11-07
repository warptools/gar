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

/// The expected treeidx bytes for gittree's `alpha` fixture tree, written by hand from the hashes gittree's tests pin.
pub(super) fn alpha_golden() -> Vec<u8> {
	fs::read(manifest_dir().join("fixtures/treeidx/alpha.treeidx"))
		.expect("the golden fixture is readable")
}

pub(super) fn hash(hex: &str) -> gittree::Hash {
	gittree::Hash::from_hex_strict(hex).expect("test hashes are well-formed")
}
