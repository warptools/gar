//! Path resolution, done textually.
//!
//! Nothing here touches the filesystem: `gar init` routinely names paths that don't exist yet,
//! and `fs::canonicalize` refuses those.
//!
//! The cost of staying textual is that ".." is folded by name rather than by inode,
//! so a ".." that crosses a symlink lands somewhere other than where the kernel would take it.
//! That caveat is also why stable std won't do this for us: `std::path::absolute` makes a path
//! absolute but deliberately keeps ".." (folding it can change which directory a path names).
//! A lexical normalize exists as `Path::normalize_lexically`, but it's nightly-only.
//! Crates like `path-clean` fold ".." but don't resolve against a base or clamp a climb past the
//! root, so they'd replace only the trivial half -- hence this small, exact function of our own.

use std::path::{Component, Path, PathBuf};

/// Resolves `p` against `base`, folding away "." and ".." components.
///
/// `base` is consulted only when `p` is relative, and is expected to be absolute itself;
/// the usual caller passes the process's current directory.
///
/// A ".." at the root resolves to the root, which is what the kernel does too.
pub fn resolve_lexical(base: &Path, p: &Path) -> PathBuf {
	// `join` discards `base` entirely when `p` is absolute, which is exactly the wanted behavior.
	let joined = base.join(p);
	let mut out = PathBuf::new();
	for comp in joined.components() {
		match comp {
			Component::CurDir => {},
			Component::ParentDir => {
				// `pop` at the root is a no-op, so climbing above the root stays at the root.
				out.pop();
			},
			c => out.push(c),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use rstest::rstest;

	#[rstest]
	// Relative input is what the base is for.
	#[case("/home/alice", "proj", "/home/alice/proj")]
	#[case("/home/alice", ".", "/home/alice")]
	#[case("/home/alice", "./proj/./deeper", "/home/alice/proj/deeper")]
	// Absolute input ignores the base.
	#[case("/home/alice", "/tmp/elsewhere", "/tmp/elsewhere")]
	// ".." folds textually, in the input and in the base's own tail.
	#[case("/home/alice", "..", "/home")]
	#[case("/home/alice", "../bob/proj", "/home/bob/proj")]
	#[case("/home/alice", "proj/../other", "/home/alice/other")]
	#[case("/home/alice", "/tmp/a/../b", "/tmp/b")]
	// Climbing past the root stays at the root, rather than producing something unusable.
	#[case("/home/alice", "../../../../..", "/")]
	// Trailing slashes are not meaningful to us.
	#[case("/home/alice", "proj/", "/home/alice/proj")]
	fn test_resolve_lexical(#[case] base: &str, #[case] p: &str, #[case] expect: &str) {
		assert_eq!(
			resolve_lexical(Path::new(base), Path::new(p)),
			PathBuf::from(expect)
		);
	}
}
