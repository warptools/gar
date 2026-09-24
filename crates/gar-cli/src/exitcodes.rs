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
//!
//! 2 is unassigned.  The gap stays: renumbering would change the interface for anyone
//! already reading these, and that's a bigger decision than tidiness.

pub const SUCCESS: i32 = 0;
pub const USAGE: i32 = 1;
pub const NO_REPO: i32 = 3;
pub const REPO_EXISTS: i32 = 4;
pub const IO_ERROR: i32 = 5;
