//! Parsing of demands files: the lists of treehashes other tools hand to `gar gc`
//! to say what must stay in stock.
//!
//! The format is a public interface of gar, specified in README_formats.md (the SSOT).
//! In short: one treehash (64 lowercase hex characters) per line;
//! blank lines and lines starting with `#` are ignored; anything else is a parse error.
//!
//! Parsing is strict and total: the first bad line fails the whole file,
//! so a caller never acts on a half-understood demands list.

use std::fmt;

/// A line that is neither a treehash, a `#` comment, nor blank.
///
/// Carries the 1-based line number; the caller knows the file name and prepends it.
#[derive(Debug, PartialEq, Eq)]
pub struct BadDemandLine {
	pub line_no: usize,
}

impl fmt::Display for BadDemandLine {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"line {}: a demand is one treehash per line (exactly 64 lowercase hex characters); blank lines and '#' comments are allowed",
			self.line_no
		)
	}
}

/// Parses the text of one demands file into the treehashes it names, in file order.
///
/// Duplicates are preserved; callers union everything into a set anyway.
pub fn parse(text: &str) -> Result<Vec<gittree::Hash>, BadDemandLine> {
	let mut out = Vec::new();
	// Split on '\n' exactly, rather than `str::lines()`: lines() quietly strips a trailing '\r',
	// which would make CRLF files parse.  The format says LF; a CRLF file is a mangled file.
	for (idx, line) in text.split('\n').enumerate() {
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		match gittree::Hash::from_hex_strict(line) {
			Some(hash) => out.push(hash),
			None => {
				return Err(BadDemandLine {
					line_no: idx + 1,
				});
			},
		}
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use rstest::rstest;

	const H1: &str = "2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce";
	const H2: &str = "2128f977414c236e7a4367508fbc095549e50ae05f26983540e86b0abd7d1e58";

	#[rstest]
	#[case::empty("", &[])]
	#[case::one_hash(&format!("{H1}\n"), &[H1])]
	#[case::no_trailing_newline(H1, &[H1])]
	#[case::two_hashes(&format!("{H1}\n{H2}\n"), &[H1, H2])]
	#[case::comments_and_blanks(&format!("# kept by the builder\n\n{H1}\n\n# more\n{H2}\n"), &[H1, H2])]
	#[case::duplicates_preserved(&format!("{H1}\n{H1}\n"), &[H1, H1])]
	fn parses(#[case] text: &str, #[case] want: &[&str]) {
		let got = parse(text).expect("should parse");
		let got: Vec<String> = got.iter().map(gittree::Hash::as_hex).collect();
		assert_eq!(got, want);
	}

	/// Strictness is the point: a demands file drives deletion,
	/// so anything not exactly understood must refuse to parse.
	#[rstest]
	#[case::uppercase(&H1.to_uppercase(), 1)]
	#[case::too_short(&H1[1..], 1)]
	#[case::too_long(&format!("{H1}a"), 1)]
	#[case::leading_space(&format!(" {H1}"), 1)]
	#[case::trailing_space(&format!("{H1} "), 1)]
	#[case::crlf(&format!("{H1}\r\n"), 1)]
	#[case::comment_not_at_column_zero(" # indented comment", 1)]
	#[case::line_number_is_the_bad_one(&format!("# fine\n{H1}\n\nnot a hash\n"), 4)]
	fn rejects(#[case] text: &str, #[case] want_line: usize) {
		let err = parse(text).expect_err("should refuse to parse");
		assert_eq!(err.line_no, want_line);
	}
}
