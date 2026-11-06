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

/// Where a tree lands in the heap under `root`, per the sharded layout.
///
/// Spelled out here rather than borrowed from the library,
/// so a change to the layout shows up as test failures.
fn treecas_entry(root: &Path, hash: &str) -> PathBuf {
	root.join(".gar/treecas")
		.join(&hash[0..2])
		.join(&hash[2..4])
		.join(hash)
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
	assert!(r.stderr.contains("already exists"), "stderr: {}", r.stderr);
	assert!(
		r.stdout.is_empty(),
		"nothing succeeded, so stdout should be empty; stdout: {}",
		r.stdout
	);
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
		treecas_entry(td.path(), hash).is_dir(),
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

/// A file squatting where a treecas entry belongs must not read as a completed add:
/// the rename fails, and returning the hash anyway would name a tree the heap lacks.
#[test]
fn add_refuses_to_commit_onto_a_treecas_entry_that_is_a_file() {
	let td = scratch("add-treecas-imposter");
	gar(td.path(), &["init"]);
	std::fs::create_dir(td.path().join("stuff")).expect("mkdir stuff");
	std::fs::write(td.path().join("stuff/a_file"), b"a file\n").expect("write a file");

	// First add succeeds and materializes the treecas entry as a directory.
	let r1 = gar(td.path(), &["add", "stuff"]);
	assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);
	let hash = r1.stdout.trim().to_owned();
	let entry = treecas_entry(td.path(), &hash);
	assert!(
		entry.is_dir(),
		"first add should have made a treecas directory"
	);

	// Replace that entry with a plain file, then add the same content again.
	std::fs::remove_dir_all(&entry).expect("remove the treecas dir");
	std::fs::write(&entry, b"not a tree\n").expect("write a file in its place");
	let r2 = gar(td.path(), &["add", "stuff"]);
	assert_ne!(
		r2.code, 0,
		"add reported success over a treecas entry that is a plain file; stdout: {}",
		r2.stdout
	);
}

/// The heap layout is an external contract: other tools read heaps directly.
/// So the paths here are spelled out literally rather than built by any helper,
/// which would change right along with the thing under test.
///
/// The two files share content, so both blobs exist either way;
/// inode identity is what shows each tree entry links to the blob whose `-x` suffix matches its mode.
#[test]
fn add_links_tree_entries_to_sharded_blobs() {
	use std::os::unix::fs::{MetadataExt, PermissionsExt};

	let td = scratch("add-layout");
	gar(td.path(), &["init"]);
	let stuff = td.path().join("stuff");
	std::fs::create_dir(&stuff).expect("mkdir stuff");
	std::fs::write(stuff.join("a_file"), b"a file\n").expect("write a file");
	std::fs::write(stuff.join("a_script"), b"a file\n").expect("write a script");
	std::fs::set_permissions(stuff.join("a_script"), PermissionsExt::from_mode(0o755))
		.expect("chmod the script");

	let r = gar(td.path(), &["add", "stuff"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	let hash = "2128f977414c236e7a4367508fbc095549e50ae05f26983540e86b0abd7d1e58";
	assert_eq!(r.stdout.trim(), hash);
	let tree = format!(".gar/treecas/21/28/{hash}");
	let blob =
		".gar/blobcas/29/09/2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce";
	let ino = |p: &str| match std::fs::metadata(td.path().join(p)) {
		Ok(m) => m.ino(),
		Err(e) => panic!("stat {p}: {e}"),
	};
	assert_eq!(
		ino(&format!("{tree}/a_file")),
		ino(blob),
		"a_file should link the plain blob"
	);
	assert_eq!(
		ino(&format!("{tree}/a_script")),
		ino(&format!("{blob}-x")),
		"a_script should link the -x blob"
	);
}

/// Copying is the default: the heap must never share an inode with the user's originals,
/// so later edits to those originals can't reach into it.
#[test]
fn add_does_not_share_inodes_with_the_originals() {
	use std::os::unix::fs::MetadataExt;

	let td = scratch("add-copies");
	gar(td.path(), &["init"]);
	std::fs::create_dir(td.path().join("stuff")).expect("mkdir stuff");
	std::fs::write(td.path().join("stuff/a_file"), b"a file\n").expect("write a file");

	let r = gar(td.path(), &["add", "stuff"]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	let stored = treecas_entry(td.path(), r.stdout.trim()).join("a_file");
	let ino = |p: &Path| std::fs::metadata(p).expect("stat").ino();
	assert_ne!(
		ino(&stored),
		ino(&td.path().join("stuff/a_file")),
		"the stored file must not share an inode with the original"
	);
}

/// Adding a tree the heap already holds makes the final rename fail, because the entry is there.
/// That's another add having gotten there first, which in a CAS is success.
#[test]
fn add_of_an_already_stored_tree_succeeds() {
	let td = scratch("add-twice");
	gar(td.path(), &["init"]);
	std::fs::create_dir(td.path().join("stuff")).expect("mkdir stuff");
	std::fs::write(td.path().join("stuff/a_file"), b"a file\n").expect("write a file");

	let r1 = gar(td.path(), &["add", "stuff"]);
	assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);
	let r2 = gar(td.path(), &["add", "stuff"]);
	assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
	assert_eq!(r1.stdout, r2.stdout);
}

/// `gar path` is the resolution boundary for other tools: they hand it a hash and use
/// the printed path, never computing the treecas layout themselves.
/// So what it prints must be exactly one line: the absolute path, usable as-is out of a pipe.
#[test]
fn path_prints_the_stored_tree_and_nothing_else() {
	let td = scratch("path-found");
	gar(td.path(), &["init"]);
	std::fs::create_dir(td.path().join("stuff")).expect("mkdir stuff");
	std::fs::write(td.path().join("stuff/a_file"), b"a file\n").expect("write a file");
	let hash = gar(td.path(), &["add", "stuff"]).stdout.trim().to_owned();

	let r = gar(td.path(), &["path", &hash]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	let printed = r
		.stdout
		.strip_suffix('\n')
		.expect("output ends with one newline");
	assert!(
		!printed.contains('\n'),
		"one line only; stdout: {:?}",
		r.stdout
	);
	let printed = Path::new(printed);
	assert!(printed.is_absolute(), "printed: {printed:?}");
	assert!(printed.is_dir(), "printed: {printed:?}");
	// Round trip: hashing what the printed path names gives back the hash asked about.
	let rehash = gar(td.path(), &["hash", arg(printed)]);
	assert_eq!(rehash.stdout.trim(), hash);
}

/// The path must come out absolute even when the repo was found by searching upward from the cwd.
#[test]
fn path_from_a_subdir_is_still_absolute() {
	let td = scratch("path-from-subdir");
	gar(td.path(), &["init"]);
	let sub = td.path().join("sub");
	std::fs::create_dir(&sub).expect("can make a subdir");
	std::fs::write(sub.join("a_file"), b"a file\n").expect("can write a file");
	let hash = gar(&sub, &["add", "."]).stdout.trim().to_owned();

	let r = gar(&sub, &["path", &hash]);
	assert_eq!(r.code, 0, "stderr: {}", r.stderr);
	let printed = Path::new(r.stdout.trim_end());
	assert!(printed.is_absolute(), "printed: {printed:?}");
	assert_eq!(printed, treecas_entry(td.path(), &hash));
}

#[test]
fn path_of_a_tree_not_in_the_heap_reports_not_in_stock() {
	let td = scratch("path-not-in-stock");
	gar(td.path(), &["init"]);
	let absent = "0000000000000000000000000000000000000000000000000000000000000000";
	let r = gar(td.path(), &["path", absent]);
	assert_eq!(r.code, 6, "stdout: {}, stderr: {}", r.stdout, r.stderr);
	assert!(
		r.stdout.is_empty(),
		"nothing resolved, so stdout stays empty; stdout: {}",
		r.stdout
	);
	assert!(
		r.stderr.contains(absent),
		"the report should name the hash asked about; stderr: {}",
		r.stderr
	);
}

/// The hash argument is strict: exactly 64 lowercase hex characters, or it's a usage error.
/// Uppercase in particular is rejected, not downcased.
#[test]
fn path_rejects_malformed_hashes_as_usage_errors() {
	let td = scratch("path-malformed");
	gar(td.path(), &["init"]);
	for bad in [
		"a".repeat(63),
		"a".repeat(65),
		"g".repeat(64),
		"A".repeat(64),
	] {
		let r = gar(td.path(), &["path", &bad]);
		assert_eq!(r.code, 1, "hash {bad:?}; stderr: {}", r.stderr);
		assert!(
			r.stdout.is_empty(),
			"hash {bad:?} resolved to something; stdout: {}",
			r.stdout
		);
	}
}

/// `path` needs a repo, same as `add`.
/// Pinned with an explicit `--repo` so no ancestor of the tempdir can satisfy a search.
#[test]
fn path_without_a_repo_reports_no_repo() {
	let td = scratch("path-no-repo");
	let r = gar(
		td.path(),
		&["--repo", arg(td.path()), "path", &"a".repeat(64)],
	);
	assert_eq!(r.code, 3, "stdout: {}, stderr: {}", r.stdout, r.stderr);
}
