//! The canonical serializer: a verified index to its one valid byte sequence.

use super::entry::{Kind, TreeIdx};
use super::format;

/// Writes the canonical bytes of an index.
///
/// Infallible, because a `TreeIdx` is already verified:
/// its order is canonical and every path fits the length field.
pub fn serialize(idx: &TreeIdx) -> Vec<u8> {
	let mut out = Vec::from(format::HEADER);
	for entry in idx.entries() {
		let path = format::decorated_path(&entry.path, entry.kind.is_dir());
		out.extend_from_slice(
			format!(
				"{:>width$} ",
				path.len(),
				width = format::LENGTH_FIELD_WIDTH
			)
			.as_bytes(),
		);
		out.extend_from_slice(&path);
		out.push(b' ');
		out.extend_from_slice(format::mode_token(&entry.kind));
		out.push(b' ');
		match &entry.kind {
			Kind::File {
				size,
				hash,
				..
			} => {
				out.extend_from_slice(format!("{size} {}", hash.as_hex()).as_bytes());
			},
			Kind::Dir {
				hash,
			} => {
				out.extend_from_slice(format::DIR_SIZE);
				out.push(b' ');
				out.extend_from_slice(hash.as_hex().as_bytes());
			},
			Kind::Symlink {
				target,
			} => {
				out.extend_from_slice(format!("{} ", target.len()).as_bytes());
				out.extend_from_slice(target);
			},
		}
		out.push(b'\n');
	}
	out
}
