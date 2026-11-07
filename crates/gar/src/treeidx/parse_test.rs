use rstest::rstest;

use super::testutil::{ALPHA_ROOT, alpha_golden, hash};
use super::{Reason, parse, serialize};

/// The golden fixture split into lines, so cases can mangle it by line:
/// index 0 is the header, and index N is entry N.
/// (Splitting on newlines is only sound because alpha has no newlines in its names.)
type Lines = Vec<String>;

fn alpha_lines() -> Lines {
	let text = String::from_utf8(alpha_golden()).expect("the golden fixture is ASCII");
	text.split_inclusive('\n').map(String::from).collect()
}

/// Replaces `from` with `to` in one line, insisting `from` is there,
/// so a case can't silently stop mangling anything if the fixture changes.
fn edit(lines: &mut Lines, line: usize, from: &str, to: &str) {
	assert_eq!(
		lines[line].matches(from).count(),
		1,
		"{from:?} should appear once in {:?}",
		lines[line]
	);
	lines[line] = lines[line].replace(from, to);
}

#[test]
fn golden_parses_and_reserializes_identically() {
	let golden = alpha_golden();
	let idx = parse(&golden).expect("the golden fixture parses");
	assert_eq!(idx.root_hash(), &hash(ALPHA_ROOT));
	assert_eq!(idx.entries().len(), 8);
	assert_eq!(serialize(&idx), golden);
}

/// Stands for any `HashMismatch` in the refusal table, which compares only reason variants.
/// (The verify tests check mismatch hashes exactly.)
fn any_mismatch() -> Reason {
	Reason::HashMismatch {
		claimed: hash(ALPHA_ROOT),
		computed: hash(ALPHA_ROOT),
	}
}

/// Each case mangles the golden fixture in one way, and must be refused for that reason, at that entry.
#[rstest]
// Header.
#[case::empty_input(|l: &mut Lines| l.clear(), Reason::BadHeader, 0)]
#[case::other_version(|l: &mut Lines| edit(l, 0, "v1", "v2"), Reason::BadHeader, 0)]
#[case::old_draft_header(|l: &mut Lines| l[0] = "# garidx v1\n".into(), Reason::BadHeader, 0)]
#[case::crlf_header(|l: &mut Lines| edit(l, 0, "\n", "\r\n"), Reason::BadHeader, 0)]
#[case::header_only(|l: &mut Lines| l.truncate(1), Reason::MissingRoot, 1)]
#[case::root_not_first(|l: &mut Lines| l.swap(1, 2), Reason::MissingRoot, 1)]
// Length field.
#[case::length_leading_zero(|l: &mut Lines| edit(l, 7, "    8", "   08"), Reason::BadLengthField, 7)]
#[case::length_left_aligned(|l: &mut Lines| edit(l, 7, "    8", "8    "), Reason::BadLengthField, 7)]
#[case::length_tab_padded(|l: &mut Lines| edit(l, 7, "    8", "\t   8"), Reason::BadLengthField, 7)]
#[case::length_too_wide(|l: &mut Lines| edit(l, 7, "    8", "     8"), Reason::BadLengthField, 7)]
#[case::length_too_narrow(|l: &mut Lines| edit(l, 7, "    8", "   8"), Reason::BadLengthField, 7)]
#[case::length_overstated(|l: &mut Lines| edit(l, 7, "    8", "    9"), Reason::MissingSpace, 7)]
#[case::length_understated(|l: &mut Lines| edit(l, 7, "    8", "    7"), Reason::MissingSpace, 7)]
// Mode.
#[case::git_style_dir_mode(|l: &mut Lines| edit(l, 2, "040000", "40000"), Reason::BadMode, 2)]
#[case::unknown_mode(|l: &mut Lines| edit(l, 7, "100644", "100664"), Reason::BadMode, 7)]
// Size field.
#[case::dir_with_size(|l: &mut Lines| edit(l, 2, " - ", " 0 "), Reason::BadSizeField, 2)]
#[case::file_with_dash(|l: &mut Lines| edit(l, 7, " 7 ", " - "), Reason::BadSizeField, 7)]
#[case::size_leading_zero(|l: &mut Lines| edit(l, 7, " 7 ", " 07 "), Reason::BadSizeField, 7)]
#[case::size_not_decimal(|l: &mut Lines| edit(l, 7, " 7 ", " 0x7 "), Reason::BadSizeField, 7)]
#[case::size_overflows(|l: &mut Lines| edit(l, 7, " 7 ", " 18446744073709551616 "), Reason::BadSizeField, 7)]
#[case::symlink_size_understated(|l: &mut Lines| edit(l, 8, " 13 ", " 12 "), Reason::MissingNewline, 8)]
#[case::symlink_size_overstated(|l: &mut Lines| edit(l, 8, " 13 ", " 14 "), Reason::Truncated, 8)]
#[case::symlink_empty_target(|l: &mut Lines| edit(l, 8, " 13 target string", " 0 "), Reason::BadSymlinkTarget, 8)]
#[case::double_space(|l: &mut Lines| edit(l, 7, " 7 ", "  7 "), Reason::BadSizeField, 7)]
// Hashes.
#[case::uppercase_hex(|l: &mut Lines| edit(l, 7, "2909489adcb", "2909489ADCB"), Reason::BadHash, 7)]
#[case::hash_too_short(|l: &mut Lines| edit(l, 7, "2909489adcb", "2909489adc"), Reason::BadHash, 7)]
#[case::hash_too_long(|l: &mut Lines| edit(l, 7, "2909489adcb", "2909489adcbb"), Reason::MissingNewline, 7)]
#[case::base58_hash(|l: &mut Lines| edit(l, 5, "4698ba4d7c51602d6a50e4fb6e150e2e06d625ba5874cde627bc6dfc357a23db", "9c4fba7ef811632cde"), Reason::BadHash, 5)]
#[case::crlf_entry(|l: &mut Lines| edit(l, 7, "\n", "\r\n"), Reason::MissingNewline, 7)]
// Path spelling.
#[case::path_not_dotted(|l: &mut Lines| edit(l, 7, "    8 ./a_file", "    6 a_file"), Reason::PathNotDotted, 7)]
#[case::absolute_path(|l: &mut Lines| edit(l, 7, "    8 ./a_file", "    7 /a_file"), Reason::PathNotDotted, 7)]
#[case::double_slash(|l: &mut Lines| edit(l, 7, "    8 ./a_file", "    9 .//a_file"), Reason::BadPathSegment, 7)]
#[case::dotdot(|l: &mut Lines| edit(l, 7, "    8 ./a_file", "   11 ./../a_file"), Reason::BadPathSegment, 7)]
#[case::empty_segment(|l: &mut Lines| edit(l, 6, "   18 ./a_dir/other", "   19 ./a_dir//other"), Reason::BadPathSegment, 6)]
#[case::dir_without_slash(|l: &mut Lines| edit(l, 2, "    8 ./a_dir/", "    7 ./a_dir"), Reason::DirPathWithoutSlash, 2)]
#[case::file_with_slash(|l: &mut Lines| edit(l, 7, "    8 ./a_file", "    9 ./a_file/"), Reason::NonDirPathWithSlash, 7)]
#[case::root_double_slash(|l: &mut Lines| edit(l, 1, "    2 ./ ", "    3 .// "), Reason::BadPathSegment, 1)]
// Tree structure and order.
#[case::orphan(|l: &mut Lines| { l.remove(3); }, Reason::Orphan, 3)]
#[case::siblings_swapped(|l: &mut Lines| l.swap(5, 6), Reason::OutOfOrder, 6)]
#[case::duplicate(|l: &mut Lines| { let d = l[7].clone(); l.insert(7, d); }, Reason::DuplicatePath, 8)]
#[case::duplicate_root(|l: &mut Lines| { let d = l[1].clone(); l.insert(2, d); }, Reason::DuplicatePath, 2)]
// Arithmetic.
#[case::lying_blob_hash(|l: &mut Lines| edit(l, 4, "4698ba4d", "4698ba4e"), any_mismatch(), 3)]
// Trailing garbage.
#[case::trailing_newline(|l: &mut Lines| l.push("\n".into()), Reason::Truncated, 9)]
#[case::trailing_line(|l: &mut Lines| l.push("garbage\n".into()), Reason::BadLengthField, 9)]
#[case::trailing_valid_looking_entry(|l: &mut Lines| { let d = l[7].replace("a_file", "z_file"); l.push(d); }, any_mismatch(), 1)]
fn refusals(#[case] mangle: fn(&mut Lines), #[case] want: Reason, #[case] want_entry: usize) {
	let mut lines = alpha_lines();
	mangle(&mut lines);
	let refusal = parse(lines.concat().as_bytes()).expect_err("should be refused");
	assert_eq!(
		std::mem::discriminant(&refusal.reason),
		std::mem::discriminant(&want),
		"refusal was: {refusal}"
	);
	assert_eq!(refusal.entry, want_entry, "refusal was: {refusal}");
}

/// Every proper prefix of a valid index must be refused:
/// those cut mid-entry are truncated, and those cut between entries lose content the root hash covers.
#[test]
fn every_truncation_is_refused() {
	let golden = alpha_golden();
	for len in 0..golden.len() {
		if let Ok(idx) = parse(&golden[..len]) {
			panic!("a {len}-byte prefix parsed, as {idx:?}");
		}
	}
}

/// Refusals point at the byte where the problem is, or for structural problems, where the entry starts.
#[test]
fn refusals_say_where() {
	let mut lines = alpha_lines();
	edit(&mut lines, 7, "2909489adcb", "2909489ADCB");
	let text = lines.concat();
	let refusal = parse(text.as_bytes()).expect_err("uppercase hex is refused");
	assert_eq!(refusal.offset, text.find("2909489ADCB"));
	assert_eq!(refusal.path.as_deref(), Some(&b"./a_file"[..]));
	assert_eq!(
		refusal.to_string(),
		format!(
			"treeidx entry 7 at byte {} (\"./a_file\"): a hash must be exactly 64 lowercase hex characters",
			text.find("2909489ADCB").unwrap_or_default()
		)
	);

	let mut lines = alpha_lines();
	lines.swap(5, 6);
	let text = lines.concat();
	let refusal = parse(text.as_bytes()).expect_err("misordered entries are refused");
	assert_eq!(refusal.offset, text.find("   18 ./a_dir/more_files"));
}

/// A path longer than the length field can express can't be written, so it can't be read either.
#[test]
fn six_digit_length_is_refused() {
	let mut text = alpha_lines().concat().into_bytes();
	text.extend_from_slice(b"100000 ./");
	text.extend_from_slice(&[b'z'; 99998]);
	text.extend_from_slice(
		b" 100644 7 2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce\n",
	);
	let refusal = parse(&text).expect_err("should be refused");
	assert_eq!(refusal.reason, Reason::MissingSpace);
	assert_eq!(refusal.entry, 9);
}
