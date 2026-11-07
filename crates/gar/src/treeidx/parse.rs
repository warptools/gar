//! The strict parser: bytes to a verified index, or a refusal.
//!
//! Reading is driven by the length fields, never by splitting on newlines:
//! paths and symlink targets may themselves contain newlines (and spaces).
//! Every field must be spelled exactly as the serializer would spell it,
//! so anything that parses also re-serializes to the identical bytes.

use super::entry::{Entry, Kind, TreeIdx};
use super::format;
use super::refusal::{Reason, Refusal};
use super::verify::verify;

/// Parses and verifies a treeidx file.
///
/// Refuses the whole input at the first deviation from the canonical format,
/// and then refuses anything `verify` would, with byte offsets attached.
pub fn parse(bytes: &[u8]) -> Result<TreeIdx, Refusal> {
	if !bytes.starts_with(format::HEADER) {
		return Err(Refusal {
			entry: 0,
			offset: Some(0),
			path: None,
			reason: Reason::BadHeader,
		});
	}
	let mut cur = Cursor {
		bytes,
		pos: format::HEADER.len(),
		ordinal: 0,
		path: None,
	};
	let mut entries = Vec::new();
	let mut starts = Vec::new();
	while cur.pos < bytes.len() {
		cur.ordinal += 1;
		cur.path = None;
		starts.push(cur.pos);
		entries.push(cur.entry()?);
	}
	verify(entries).map_err(|mut refusal| {
		// An entry ordinal past the last one (as with no entries at all) is a problem at the end of input.
		let start = refusal.entry.checked_sub(1).and_then(|i| starts.get(i));
		refusal.offset = Some(start.copied().unwrap_or(bytes.len()));
		refusal
	})
}

struct Cursor<'a> {
	bytes: &'a [u8],
	pos: usize,
	/// The 1-based ordinal of the entry being read.
	ordinal: usize,
	/// The current entry's path as written, once it's been read, for refusal messages.
	path: Option<Vec<u8>>,
}

impl<'a> Cursor<'a> {
	fn refuse(&self, offset: usize, reason: Reason) -> Refusal {
		Refusal {
			entry: self.ordinal,
			offset: Some(offset),
			path: self.path.clone(),
			reason,
		}
	}

	fn take(&mut self, n: usize) -> Result<&'a [u8], Refusal> {
		let Some(taken) = self.bytes.get(self.pos..self.pos.saturating_add(n)) else {
			return Err(self.refuse(self.bytes.len(), Reason::Truncated));
		};
		self.pos += n;
		Ok(taken)
	}

	fn expect(&mut self, want: u8, reason: Reason) -> Result<(), Refusal> {
		let at = self.pos;
		if self.take(1)? != [want] {
			return Err(self.refuse(at, reason));
		}
		Ok(())
	}

	/// Takes the bytes up to (not including) the next space, and consumes the space.
	fn take_word(&mut self) -> Result<&'a [u8], Refusal> {
		let rest = &self.bytes[self.pos..];
		let Some(len) = rest.iter().position(|&b| b == b' ') else {
			return Err(self.refuse(self.bytes.len(), Reason::Truncated));
		};
		self.pos += len + 1;
		Ok(&rest[..len])
	}

	fn take_hash(&mut self) -> Result<gittree::Hash, Refusal> {
		let at = self.pos;
		let hex = self.take(64)?;
		std::str::from_utf8(hex)
			.ok()
			.and_then(gittree::Hash::from_hex_strict)
			.ok_or_else(|| self.refuse(at, Reason::BadHash))
	}

	fn size_number(&self, at: usize, field: &[u8]) -> Result<u64, Refusal> {
		parse_decimal(field).ok_or_else(|| self.refuse(at, Reason::BadSizeField))
	}

	fn take_mode(&mut self) -> Result<Mode, Refusal> {
		let at = self.pos;
		let mode = self.take(6)?;
		if mode == format::MODE_FILE {
			Ok(Mode::File {
				executable: false,
			})
		} else if mode == format::MODE_EXECUTABLE {
			Ok(Mode::File {
				executable: true,
			})
		} else if mode == format::MODE_DIR {
			Ok(Mode::Dir)
		} else if mode == format::MODE_SYMLINK {
			Ok(Mode::Symlink)
		} else {
			Err(self.refuse(at, Reason::BadMode))
		}
	}

	fn entry(&mut self) -> Result<Entry, Refusal> {
		let at = self.pos;
		let path_len = parse_length_field(self.take(format::LENGTH_FIELD_WIDTH)?)
			.ok_or_else(|| self.refuse(at, Reason::BadLengthField))?;
		self.expect(b' ', Reason::MissingSpace)?;

		let path_at = self.pos;
		let path = self.take(path_len)?;
		self.path = Some(path.to_vec());
		self.expect(b' ', Reason::MissingSpace)?;

		let mode = self.take_mode()?;
		self.expect(b' ', Reason::MissingSpace)?;

		let size_at = self.pos;
		let size_field = self.take_word()?;
		let kind = match mode {
			Mode::File {
				executable,
			} => Kind::File {
				executable,
				size: self.size_number(size_at, size_field)?,
				hash: self.take_hash()?,
			},
			Mode::Dir => {
				if size_field != format::DIR_SIZE {
					return Err(self.refuse(size_at, Reason::BadSizeField));
				}
				Kind::Dir {
					hash: self.take_hash()?,
				}
			},
			Mode::Symlink => {
				let size = self.size_number(size_at, size_field)?;
				// A size too big for memory can't be satisfied by the input either.
				let size = usize::try_from(size).unwrap_or(usize::MAX);
				Kind::Symlink {
					target: self.take(size)?.to_vec(),
				}
			},
		};
		self.expect(b'\n', Reason::MissingNewline)?;

		let bare = bare_path(path, kind.is_dir()).map_err(|reason| self.refuse(path_at, reason))?;
		Ok(Entry {
			path: bare.to_vec(),
			kind,
		})
	}
}

/// The entry kinds as the mode field names them, before the rest of the entry is read.
#[derive(Clone, Copy)]
enum Mode {
	File {
		executable: bool,
	},
	Dir,
	Symlink,
}

/// Strips a path as written down to its bare form, requiring exactly the decoration `format::decorated_path` adds.
fn bare_path(path: &[u8], is_dir: bool) -> Result<&[u8], Reason> {
	let Some(rest) = path.strip_prefix(b"./") else {
		return Err(Reason::PathNotDotted);
	};
	if !is_dir {
		return match rest.ends_with(b"/") {
			true => Err(Reason::NonDirPathWithSlash),
			false => Ok(rest),
		};
	}
	if rest.is_empty() {
		return Ok(rest);
	}
	match rest.strip_suffix(b"/") {
		None => Err(Reason::DirPathWithoutSlash),
		// "./" plus a lone "/" would otherwise strip down to the root's own bare path.
		Some(b"") => Err(Reason::BadPathSegment),
		Some(bare) => Ok(bare),
	}
}

/// Parses the fixed-width length field: spaces, then a canonical decimal number, right-aligned.
fn parse_length_field(field: &[u8]) -> Option<usize> {
	let padding = field.iter().take_while(|&&b| b == b' ').count();
	usize::try_from(parse_decimal(&field[padding..])?).ok()
}

/// Parses a decimal number as the format writes them: ASCII digits only, no leading zeros, no overflow.
fn parse_decimal(digits: &[u8]) -> Option<u64> {
	if digits.is_empty() || (digits[0] == b'0' && digits.len() > 1) {
		return None;
	}
	digits.iter().try_fold(0u64, |n, &d| {
		if !d.is_ascii_digit() {
			return None;
		}
		n.checked_mul(10)?.checked_add(u64::from(d - b'0'))
	})
}
