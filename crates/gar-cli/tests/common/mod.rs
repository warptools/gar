//! Helpers shared by the end-to-end test files: spawning the real `gar` binary,
//! scratch dirs, and the heap layout paths the tests assert about.

// Each test file compiles this module as its own crate, and not every file uses every helper;
// that's not dead code, just partial use per compilation.
#![allow(dead_code)]

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

pub struct Run {
	pub code: i32,
	pub stdout: String,
	pub stderr: String,
}

/// Runs gar with the given args and working directory, capturing everything about the result.
pub fn gar(cwd: &Path, args: &[&str]) -> Run {
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

/// Like `Run`, but with stdout kept as raw bytes, for the commands that speak tar on it.
pub struct RunBytes {
	pub code: i32,
	pub stdout: Vec<u8>,
	pub stderr: String,
}

/// Runs gar for the commands that speak tar over the pipes: stdout is captured raw,
/// and stdin comes from a file when given.
///
/// A file rather than an in-memory pipe, because a test writing one pipe while the child
/// fills another is a deadlock waiting on a buffer size; `< file` has no such cliff.
pub fn gar_bytes(cwd: &Path, args: &[&str], stdin_file: Option<&Path>) -> RunBytes {
	let mut cmd = Command::new(gar_bin());
	cmd.args(args).current_dir(cwd);
	cmd.stdin(match stdin_file {
		Some(p) => std::process::Stdio::from(std::fs::File::open(p).expect("the stdin file opens")),
		None => std::process::Stdio::null(),
	});
	let out = cmd.output().expect("gar binary is executable");
	RunBytes {
		code: out
			.status
			.code()
			.expect("gar exited rather than being signalled"),
		stdout: out.stdout,
		stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
	}
}

/// A scratch dir to work in.  Named after the test, so a leaked one is traceable.
pub fn scratch(name: &str) -> tempdir::TempDir {
	tempdir::TempDir::new(&format!("gar-test-{name}-")).expect("can make a tempdir")
}

/// Where a tree lands in the heap under `root`, per the sharded layout.
///
/// Spelled out here rather than borrowed from the library,
/// so a change to the layout shows up as test failures.
pub fn treecas_entry(root: &Path, hash: &str) -> PathBuf {
	root.join(".gar/treecas")
		.join(&hash[0..2])
		.join(&hash[2..4])
		.join(hash)
}

/// Where a blob lands in the heap under `root`, per the sharded layout.  Same spirit as `treecas_entry`.
/// The hash is the blob hash (what `gar hash` prints for the file); executable blobs carry a `-x` suffix.
pub fn blobcas_entry(root: &Path, hash: &str) -> PathBuf {
	root.join(".gar/blobcas")
		.join(&hash[0..2])
		.join(&hash[2..4])
		.join(hash)
}

/// Where a tree's index file lands in the heap under `root`, per the sharded layout.  Same spirit as `treecas_entry`.
pub fn treeidx_entry(root: &Path, hash: &str) -> PathBuf {
	root.join(".gar/treeidx")
		.join(&hash[0..2])
		.join(&hash[2..4])
		.join(hash)
}

/// Paths from a tempdir are passed as args; this is the unwrap for that.
pub fn arg(p: &Path) -> &str {
	p.to_str().expect("tempdir paths are utf8")
}
