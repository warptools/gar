//! End-to-end tests: these spawn the real `gar` binary and look at what a user would see.
//!
//! Exit codes and messages are part of the interface of a CLI tool,
//! so they're worth pinning down here rather than only testing the library innards.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Locates the `gar` binary to exercise.
///
/// Cargo builds bin targets before running integration tests, and puts the test binary
/// in `{target}/{profile}/deps/` -- so the bin we want is two levels up, in `{target}/{profile}/`.
///
/// Deliberately found at runtime rather than with `env!("CARGO_BIN_EXE_gar")`:
/// compile-time environment capture leans on cargo's cache invalidation to stay true,
/// and this route also follows a debug or release build without being told which we're in.
fn gar_bin() -> PathBuf {
	let test_bin = std::env::current_exe().expect("the running test binary has a path");
	let profile_dir = test_bin
		.parent()
		.and_then(Path::parent)
		.expect("test binary lives at {target}/{profile}/deps/{name}");
	let bin = profile_dir.join("gar");
	assert!(bin.exists(), "no gar binary at {bin:?}");
	bin
}

struct Run {
	code: i32,
	stdout: String,
	stderr: String,
}

/// Runs gar with the given args and working directory, capturing everything about the result.
fn gar(cwd: &Path, args: &[&str]) -> Run {
	let out = Command::new(gar_bin())
		.args(args)
		.current_dir(cwd)
		.output()
		.expect("gar binary is executable");
	Run {
		code: out
			.status
			.code()
			.expect("gar exited rather than being signalled"),
		stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
		stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
	}
}

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// Paths from a tempdir are passed as args; this is the unwrap for that.
fn arg(p: &Path) -> &str {
	p.to_str().expect("tempdir paths are utf8")
}

#[test]
fn init_in_cwd() {
	let td = scratch("init-in-cwd");
	let r = gar(td.path(), &["init"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(td.path().join(".gar/blobcas").is_dir());
	assert!(td.path().join(".gar/treecas").is_dir());
	assert!(td.path().join(".gar/treeidx").is_dir());
}

#[test]
fn init_twice_reports_the_existing_repo() {
	let td = scratch("init-twice");
	gar(td.path(), &["init"]);
	let r = gar(td.path(), &["init"]);
	assert_eq!(r.code, 4, "stderr: {}", r.stderr);
	assert!(r.stdout.contains("already exists"), "stdout: {}", r.stdout);
}

/// Searching upward from the cwd is still how a repo is found when `--repo` is absent.
#[test]
fn add_from_a_subdir_finds_the_ancestor_repo() {
	let td = scratch("add-from-subdir");
	gar(td.path(), &["init"]);
	let sub = td.path().join("sub");
	std::fs::create_dir(&sub).expect("can make a subdir");
	std::fs::write(sub.join("a_file"), b"a file\n").expect("can write a file");

	let r = gar(&sub, &["add", "."]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	let hash = r.stdout.trim();
	assert!(
		td.path().join(".gar/treecas").join(hash).is_dir(),
		"the tree should have landed in the ancestor repo; stdout: {}",
		r.stdout
	);
}

/// A typo'd `--repo` path should not conjure a directory chain into existence.
#[test]
fn init_does_not_create_missing_ancestors() {
	let td = scratch("init-no-mkdirp");
	let target = td.path().join("no/such/deep/place");

	let r = gar(td.path(), &["--repo", arg(&target), "init"]);
	assert_ne!(r.code, 0, "should have refused; stdout: {}", r.stdout);
	assert!(
		!td.path().join("no").exists(),
		"not one directory of the missing chain should have been created"
	);
	assert!(
		r.stderr.contains("no/such/deep"),
		"the message should name the directory that's missing; stderr: {}",
		r.stderr
	);
}

/// One level of directory-making is fine: it's the chain that's the problem.
#[test]
fn init_creates_the_named_directory() {
	let td = scratch("init-one-level");
	let target = td.path().join("fresh");

	let r = gar(td.path(), &["--repo", arg(&target), "init"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(target.join(".gar/blobcas").is_dir());
}

/// `--repo` names the repo exactly.  It is not a place to start searching from.
#[test]
fn explicit_repo_is_not_a_search_start() {
	let td = scratch("explicit-no-search");
	gar(td.path(), &["init"]);
	let sub = td.path().join("sub");
	std::fs::create_dir(&sub).expect("can make a subdir");

	let r = gar(td.path(), &["--repo", arg(&sub), "init"]);
	assert_eq!(
		r.code, 0,
		"an explicit --repo means that exact place, even nested inside another repo; stderr: {}",
		r.stderr
	);
	assert!(sub.join(".gar/blobcas").is_dir());
}

/// The hazard this closes: an explicit `--repo` that isn't a repo must never
/// quietly write into some ancestor's heap instead.
#[test]
fn add_with_an_explicit_nonrepo_errors_rather_than_finding_an_ancestor() {
	let td = scratch("explicit-nonrepo");
	gar(td.path(), &["init"]);
	let work = td.path().join("work");
	std::fs::create_dir(&work).expect("can make a subdir");
	std::fs::write(work.join("a_file"), b"a file\n").expect("can write a file");

	let r = gar(td.path(), &["--repo", arg(&work), "add", "work"]);
	assert_eq!(r.code, 3, "stdout: {}, stderr: {}", r.stdout, r.stderr);
	let leaked: Vec<_> = std::fs::read_dir(td.path().join(".gar/treecas"))
		.expect("the ancestor repo exists")
		.map(|e| e.expect("readable dirent").file_name())
		.collect();
	assert!(
		leaked.is_empty(),
		"nothing should have been written to the ancestor repo, found: {leaked:?}"
	);
}

/// A relative `--repo` resolves against the cwd, and is reported as the absolute place it landed.
#[test]
fn relative_explicit_repo_resolves_against_cwd() {
	let td = scratch("relative-repo");
	let sub = td.path().join("sub");
	std::fs::create_dir(&sub).expect("can make a subdir");

	let r = gar(&sub, &["--repo", "../sub", "init"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	assert!(sub.join(".gar/blobcas").is_dir());
	assert!(
		!r.stdout.contains(".."),
		"the reported path should be resolved, not echoed back; stdout: {}",
		r.stdout
	);
}
