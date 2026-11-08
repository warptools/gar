//! The exit codes `gar` returns, and what each one means.
//!
//! Exit codes are part of a CLI's interface: scripts branch on them, and the end-to-end
//! tests pin them.  This module is where they're decided; everywhere else refers to these names.
//!
//! | code | name          | meaning                                                  |
//! |------|---------------|----------------------------------------------------------|
//! | 0    | `SUCCESS`     | the command did what was asked                            |
//! | 1    | `USAGE`       | the arguments didn't parse                                |
//! | 3    | `NO_REPO`     | no gar repo where one was needed                          |
//! | 4    | `REPO_EXISTS` | `init` found a repo already present                       |
//! | 5    | `IO_ERROR`    | something went wrong on the filesystem                    |
//! | 6    | `NOT_IN_STOCK`| `path`, `export`, or `index` named a tree the heap doesn't hold (`index` still processed the rest); or `fetch`'s remote doesn't hold the tree: no index for it, or a blob its index names is missing |
//! | 7    | `DEMAND_NOT_IN_STOCK` | `gc` was told to keep a tree the heap doesn't hold (the rest of the gc still ran) |
//! | 8    | `HASH_MISMATCH` | `import --expect` stored the stream, but it hashed to something else (the stored tree stays); or `fetch` refused what the remote served: an index or blob that doesn't verify against what was asked for (nothing unverified lands) |
//! | 9    | `BAD_TAR`     | `import` refused the tar stream: an unsupported entry type, or a path that is absolute or `..`-escaping |
//! | 10   | `CORRUPTION_FOUND` | `repair` found corruption, and moved the offenders into quarantine; or `index` found trees that don't hash to their names, and wrote no index for them |
//!
//! An escape attempted by writing through a symlink is refused too, but by the tar unpacker
//! underneath rather than by gar's own path checks, so it surfaces as 5.
//!
//! 2 is unassigned.  The gap stays: renumbering would change the interface for anyone
//! already reading these, and that's a bigger decision than tidiness.

pub const SUCCESS: i32 = 0;
pub const USAGE: i32 = 1;
pub const NO_REPO: i32 = 3;
pub const REPO_EXISTS: i32 = 4;
pub const IO_ERROR: i32 = 5;
pub const NOT_IN_STOCK: i32 = 6;
pub const DEMAND_NOT_IN_STOCK: i32 = 7;
pub const HASH_MISMATCH: i32 = 8;
pub const BAD_TAR: i32 = 9;
pub const CORRUPTION_FOUND: i32 = 10;
