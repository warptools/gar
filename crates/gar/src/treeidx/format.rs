//! The fixed tokens and spelling rules of the treeidx byte format,
//! shared by the serializer and the parser so they can't drift apart.

use super::entry::Kind;

/// The first line of every treeidx file, newline included.
pub(super) const HEADER: &[u8] = b"treeidx v1\n";

/// The path length field is exactly this many bytes: decimal, right-aligned, space-padded.
pub(super) const LENGTH_FIELD_WIDTH: usize = 5;

/// The longest serialized path the length field can express.
pub(super) const MAX_PATH_LEN: usize = 99999;

pub(super) const MODE_FILE: &[u8; 6] = b"100644";
pub(super) const MODE_EXECUTABLE: &[u8; 6] = b"100755";
pub(super) const MODE_DIR: &[u8; 6] = b"040000";
pub(super) const MODE_SYMLINK: &[u8; 6] = b"120000";

/// The size field of directory entries, which have no size.
pub(super) const DIR_SIZE: &[u8] = b"-";

pub(super) fn mode_token(kind: &Kind) -> &'static [u8; 6] {
	match kind {
		Kind::File {
			executable: false,
			..
		} => MODE_FILE,
		Kind::File {
			executable: true,
			..
		} => MODE_EXECUTABLE,
		Kind::Dir {
			..
		} => MODE_DIR,
		Kind::Symlink {
			..
		} => MODE_SYMLINK,
	}
}

/// Spells a bare path the way the format writes it:
/// a `./` prefix always, and a trailing `/` for directories other than the root (which is just `./`).
pub(super) fn decorated_path(bare: &[u8], is_dir: bool) -> Vec<u8> {
	let mut out = Vec::with_capacity(bare.len() + 3);
	out.extend_from_slice(b"./");
	out.extend_from_slice(bare);
	if is_dir && !bare.is_empty() {
		out.push(b'/');
	}
	out
}
