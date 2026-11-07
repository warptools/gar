use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::PathBuf;

use super::Remote;

#[test]
fn classifies_by_scheme_prefix() {
	for (base, want) in [
		(
			"http://example.com/heap",
			Remote::Http("http://example.com/heap".into()),
		),
		(
			"https://example.com/heap/",
			Remote::Http("https://example.com/heap/".into()),
		),
		(
			"/srv/heap/.gar",
			Remote::Local(PathBuf::from("/srv/heap/.gar")),
		),
		("heap/.gar", Remote::Local(PathBuf::from("heap/.gar"))),
		// Only the exact prefixes count; anything else is a path, however URL-ish.
		(
			"http:/one-slash",
			Remote::Local(PathBuf::from("http:/one-slash")),
		),
		(
			"ftp://example.com/heap",
			Remote::Local(PathBuf::from("ftp://example.com/heap")),
		),
	] {
		let got = Remote::parse(OsStr::new(base)).expect("classifiable");
		assert_eq!(got, want, "classifying {base:?}");
	}
}

#[test]
fn a_local_path_needs_no_utf8() {
	let base = OsStr::from_bytes(b"/srv/\xffheap");
	assert_eq!(
		Remote::parse(base).expect("classifiable"),
		Remote::Local(PathBuf::from(base))
	);
}

#[test]
fn a_url_that_isnt_utf8_is_refused() {
	let err = Remote::parse(OsStr::from_bytes(b"http://example.com/\xff")).expect_err("not a URL");
	assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}
