use rstest::rstest;

use super::testutil::{ALPHA_ROOT, alpha_golden, hash};
use super::{Entry, Kind, Reason, parse, serialize, verify};

/// Shorthand for the table cases' closures, which rstest needs typed.
type Es = Vec<Entry>;

/// Alpha's entries, in order:
/// 1 `./`, 2 `./a_dir/`, 3 `./a_dir/deeper/`, 4 `./a_dir/deeper/samefile`,
/// 5 `./a_dir/more_files`, 6 `./a_dir/other_file`, 7 `./a_file`, 8 `./a_symlink`.
fn alpha_entries() -> Vec<Entry> {
	parse(&alpha_golden())
		.expect("the golden fixture parses")
		.into_entries()
}

const DEEPER: &str = "9897054d9f01c666ac1371d3e0a022a67b5df59ddb1608e8165a3b1fa22da706";
const SOME_HASH: &str = "2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce";

fn file(path: &[u8]) -> Entry {
	Entry {
		path: path.to_vec(),
		kind: Kind::File {
			executable: false,
			size: 7,
			hash: hash(SOME_HASH),
		},
	}
}

fn symlink(path: &[u8], target: &[u8]) -> Entry {
	Entry {
		path: path.to_vec(),
		kind: Kind::Symlink {
			target: target.to_vec(),
		},
	}
}

#[test]
fn alpha_verifies_to_its_root() {
	let idx = verify(alpha_entries()).expect("alpha verifies");
	assert_eq!(idx.root_hash(), &hash(ALPHA_ROOT));
}

/// Each case breaks one rule in otherwise-valid entries, and must be refused for that reason, at that entry.
/// Structural rules are checked as each entry arrives, so these refusals come before any hash comparison.
#[rstest]
#[case::empty(|e: &mut Es| e.clear(), Reason::MissingRoot, 1)]
#[case::root_missing(|e: &mut Es| { e.remove(0); }, Reason::MissingRoot, 1)]
#[case::root_not_a_dir(|e: &mut Es| e[0] = file(b""), Reason::MissingRoot, 1)]
#[case::second_root(|e: &mut Es| { let root = e[0].clone(); e.insert(1, root); }, Reason::DuplicatePath, 2)]
#[case::duplicate_file(|e: &mut Es| { let f = e[6].clone(); e.insert(7, f); }, Reason::DuplicatePath, 8)]
// A file and a dir with the same name don't collide in gitwise order (`a_dir` < `a_dir/`), so only the duplicate check sees this.
#[case::file_and_dir_same_name(|e: &mut Es| e.insert(1, file(b"a_dir")), Reason::DuplicatePath, 3)]
#[case::dot_segment(|e: &mut Es| e[6].path = b"./a_file".to_vec(), Reason::BadPathSegment, 7)]
#[case::dotdot_segment(|e: &mut Es| e[6].path = b"../a_file".to_vec(), Reason::BadPathSegment, 7)]
#[case::dotdot_inside(|e: &mut Es| e[5].path = b"a_dir/../other_file".to_vec(), Reason::BadPathSegment, 6)]
#[case::empty_segment(|e: &mut Es| e[5].path = b"a_dir//other_file".to_vec(), Reason::BadPathSegment, 6)]
#[case::absolute(|e: &mut Es| e[6].path = b"/a_file".to_vec(), Reason::BadPathSegment, 7)]
#[case::trailing_slash(|e: &mut Es| e[6].path = b"a_file/".to_vec(), Reason::BadPathSegment, 7)]
#[case::nul_in_name(|e: &mut Es| e[6].path = b"a_f\0ile".to_vec(), Reason::BadPathSegment, 7)]
#[case::path_too_long(|e: &mut Es| e.push(file(&[b'z'; 99998])), Reason::PathTooLong, 9)]
#[case::symlink_empty(|e: &mut Es| e[7] = symlink(b"a_symlink", b""), Reason::BadSymlinkTarget, 8)]
#[case::symlink_nul(|e: &mut Es| e[7] = symlink(b"a_symlink", b"tar\0get"), Reason::BadSymlinkTarget, 8)]
#[case::orphan(|e: &mut Es| { e.remove(2); }, Reason::Orphan, 3)]
#[case::parent_is_a_file(|e: &mut Es| e.insert(7, file(b"a_file/inner")), Reason::Orphan, 8)]
// Moving an existing entry out of its dir would make the dir's hash wrong as it closes, which is found first;
// a new entry under the closed dir isolates the ordering problem.
#[case::parent_already_closed(|e: &mut Es| e.insert(7, file(b"a_dir/zzz")), Reason::Orphan, 8)]
#[case::siblings_swapped(|e: &mut Es| e.swap(4, 5), Reason::OutOfOrder, 6)]
#[case::subtree_after_its_sibling(|e: &mut Es| { let f = e.remove(4); e.insert(2, f); }, Reason::OutOfOrder, 4)]
fn refusals(#[case] mangle: fn(&mut Vec<Entry>), #[case] want: Reason, #[case] want_entry: usize) {
	let mut entries = alpha_entries();
	mangle(&mut entries);
	let refusal = verify(entries).expect_err("should be refused");
	assert_eq!(refusal.reason, want, "refusal was: {refusal}");
	assert_eq!(refusal.entry, want_entry, "refusal was: {refusal}");
}

/// A lie anywhere -- a dir's own claim, or any child's hash beneath it -- surfaces as a mismatch
/// at the innermost directory whose claim no longer recomputes.
#[rstest]
#[case::dir_claim(|e: &mut Es| e[2].kind = Kind::Dir { hash: hash(SOME_HASH) }, 3, Some(SOME_HASH), Some(DEEPER))]
#[case::root_claim(|e: &mut Es| e[0].kind = Kind::Dir { hash: hash(SOME_HASH) }, 1, Some(SOME_HASH), Some(ALPHA_ROOT))]
#[case::blob_hash(|e: &mut Es| if let Kind::File { hash: h, .. } = &mut e[3].kind { *h = hash(SOME_HASH) }, 3, Some(DEEPER), None)]
#[case::exec_bit(|e: &mut Es| if let Kind::File { executable, .. } = &mut e[3].kind { *executable = true }, 3, Some(DEEPER), None)]
#[case::symlink_target(|e: &mut Es| e[7] = symlink(b"a_symlink", b"elsewhere"), 1, Some(ALPHA_ROOT), None)]
#[case::name(|e: &mut Es| e[6].path = b"a_filf".to_vec(), 1, Some(ALPHA_ROOT), None)]
fn hash_mismatches(
	#[case] mangle: fn(&mut Vec<Entry>),
	#[case] want_entry: usize,
	#[case] want_claimed: Option<&str>,
	#[case] want_computed: Option<&str>,
) {
	let mut entries = alpha_entries();
	mangle(&mut entries);
	let refusal = verify(entries).expect_err("should be refused");
	assert_eq!(refusal.entry, want_entry, "refusal was: {refusal}");
	let Reason::HashMismatch {
		claimed,
		computed,
	} = &refusal.reason
	else {
		panic!("expected a hash mismatch; refusal was: {refusal}");
	};
	if let Some(want) = want_claimed {
		assert_eq!(claimed, &hash(want));
	}
	if let Some(want) = want_computed {
		assert_eq!(computed, &hash(want));
	}
	assert_ne!(claimed, computed);
}

/// The longest path the length field can express (99999 bytes, `./` included) round-trips;
/// one byte more is refused by verification.  (Parsing can't even express it; see the parse tests.)
#[test]
fn path_length_limit() {
	let one_file_tree = |name: &[u8]| {
		let mut acc = gittree::TreeHashAccumulator::new(1);
		acc.append_file(name, &hash(SOME_HASH));
		vec![
			Entry {
				path: Vec::new(),
				kind: Kind::Dir {
					hash: acc.finish(),
				},
			},
			file(name),
		]
	};

	let idx = verify(one_file_tree(&[b'z'; 99997])).expect("a 99999-byte path fits");
	let bytes = serialize(&idx);
	assert!(
		bytes.windows(7).any(|w| w == b"99999 ."),
		"the length field should be full width"
	);
	assert_eq!(parse(&bytes).expect("and parses back"), idx);

	let refusal =
		verify(one_file_tree(&[b'z'; 99998])).expect_err("a 100000-byte path does not fit");
	assert_eq!(refusal.reason, Reason::PathTooLong);
}
