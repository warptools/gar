mod cmds;
mod exitcodes;
mod paths;

use gar::add;
use gar::demands;
use gar::export;
use gar::gc;
use gar::import;
use gar::index_backfill;
use gar::repair;
use gar::repo;

mod clap {
	pub use clap::Parser;
	pub use clap::error::ErrorKind;
}
use clap::Parser;

use std::io;
use std::io::Write as _;
use std::path::PathBuf;
use std::process;

fn main() {
	// Results go on stdout; diagnostics go on stderr.
	// Whoever reads the hash `gar add` prints out of a pipe should get the hash, and nothing else.
	let root_args = match cmds::Root::try_parse_from(std::env::args_os()) {
		Ok(args) => args,
		Err(e)
			if matches!(
				e.kind(),
				clap::ErrorKind::DisplayHelp | clap::ErrorKind::DisplayVersion
			) =>
		{
			println!("{e}");
			process::exit(exitcodes::SUCCESS);
		},
		Err(e) => {
			eprintln!("{e}");
			process::exit(exitcodes::USAGE);
		},
	};

	// The hash subcommand is repo-independent; handle it before any repo search.
	if let cmds::Subcommands::Hash(args) = &root_args.subcommand {
		match gittree::hash_of_path(&args.path) {
			Ok(hash) => {
				println!("{}", hash.as_hex());
				process::exit(exitcodes::SUCCESS);
			},
			Err(e) => {
				eprintln!("{}", e);
				process::exit(exitcodes::IO_ERROR);
			},
		}
	}
	let cwd = std::env::current_dir().expect("must be able to find cwd");

	// Where's the heap?
	// An explicit `--repo` names the directory exactly: if the user said a specific place,
	// that's the place, and no searching happens.
	// Without one, search upward from the cwd, falling back to the cwd itself
	// as where a new repo would go.
	let repo_dir = match &root_args.repo {
		Some(p) => paths::resolve_lexical(&cwd, p),
		None => match repo::find_repo_dir(&cwd) {
			Ok(Some(dir)) => dir,
			Ok(None) => cwd,
			Err(e) => {
				eprintln!("{e}");
				process::exit(exitcodes::IO_ERROR);
			},
		},
	};
	let repo = repo::Repo::new(&repo_dir);

	match root_args.subcommand {
		cmds::Subcommands::Init(_) => {
			if repo.exists() {
				eprintln!("gar repo already exists at {:?}", repo.repo_path());
				process::exit(exitcodes::REPO_EXISTS);
			}
			match repo::init_at(&repo_dir) {
				Ok(r) => println!("gar repo created at {:?}", r.repo_path()),
				Err(e) => {
					eprintln!("{e}");
					process::exit(exitcodes::IO_ERROR);
				},
			}
			process::exit(exitcodes::SUCCESS);
		},
		cmds::Subcommands::Add(args) => {
			require_repo(&repo, &root_args.repo);
			// Restores any subdirectory that's gone missing since the repo was made.
			if let Err(e) = repo.create() {
				eprintln!("{e}");
				process::exit(exitcodes::IO_ERROR);
			}
			// Copy is the only mode the CLI offers: it's the one that's safe without
			// promises about what happens to the originals afterward.
			match add::add(&repo, args.path, add::FaithMode::Copy) {
				Ok(hash) => {
					println!("{}", hash.as_hex());
					process::exit(exitcodes::SUCCESS);
				},
				Err(e) => {
					eprintln!("{}", e);
					process::exit(exitcodes::IO_ERROR);
				},
			}
		},
		cmds::Subcommands::Path(args) => {
			require_repo(&repo, &root_args.repo);
			let tree_path = repo.tree_path(&args.hash);
			// Presence means the entry directory exists; no deeper verification happens here.
			if !tree_path.is_dir() {
				eprintln!("tree {} is not in this heap", args.hash.as_hex());
				process::exit(exitcodes::NOT_IN_STOCK);
			}
			// The path is absolute because repo_dir always is: every route to it starts from the cwd.
			println!("{}", tree_path.display());
			process::exit(exitcodes::SUCCESS);
		},
		cmds::Subcommands::Gc(args) => {
			require_repo(&repo, &root_args.repo);
			// No `repo.create()` here, unlike add: a recreated empty treecas would read as
			// "no tree references any blob", and the sweep would empty the blobcas.
			// Every demands file is read and parsed in full before anything is deleted:
			// a gc driven by a half-understood demands list must not get to touch the heap.
			let mut demanded = std::collections::HashSet::new();
			for file in &args.demands {
				let text = match std::fs::read_to_string(file) {
					Ok(text) => text,
					Err(e) => {
						eprintln!("gc: reading demands file {:?}: {}", file, e);
						process::exit(exitcodes::IO_ERROR);
					},
				};
				match demands::parse(&text) {
					Ok(hashes) => demanded.extend(hashes),
					Err(e) => {
						eprintln!("gc: demands file {:?}: {}", file, e);
						process::exit(exitcodes::USAGE);
					},
				}
			}
			match gc::gc(&repo, &demanded) {
				Ok(report) => {
					eprintln!(
						"gc: kept {} trees; removed {} trees, {} indexes, and {} blobs",
						report.trees_kept,
						report.trees_removed,
						report.indexes_removed,
						report.blobs_removed
					);
					if !report.absent_demands.is_empty() {
						for hash in &report.absent_demands {
							eprintln!("gc: demanded tree {} is not in this heap", hash.as_hex());
						}
						process::exit(exitcodes::DEMAND_NOT_IN_STOCK);
					}
					process::exit(exitcodes::SUCCESS);
				},
				Err(e) => {
					eprintln!("{e}");
					process::exit(exitcodes::IO_ERROR);
				},
			}
		},
		cmds::Subcommands::Export(args) => {
			require_repo(&repo, &root_args.repo);
			// The same presence check, diagnostic, and exit as `path`:
			// asking to export an absent tree is the same misunderstanding as asking where it is.
			if !repo.tree_path(&args.hash).is_dir() {
				eprintln!("tree {} is not in this heap", args.hash.as_hex());
				process::exit(exitcodes::NOT_IN_STOCK);
			}
			// Stdout carries only tar bytes; anything else said here goes to stderr.
			let mut out = io::BufWriter::new(io::stdout().lock());
			let result = export::export(&repo, &args.hash, &mut out).and_then(|()| out.flush());
			match result {
				Ok(()) => process::exit(exitcodes::SUCCESS),
				Err(e) => {
					eprintln!("export: {e}");
					process::exit(exitcodes::IO_ERROR);
				},
			}
		},
		cmds::Subcommands::Import(args) => {
			require_repo(&repo, &root_args.repo);
			// Restores any subdirectory that's gone missing since the repo was made, same as add.
			if let Err(e) = repo.create() {
				eprintln!("{e}");
				process::exit(exitcodes::IO_ERROR);
			}
			match import::import(&repo, io::stdin().lock()) {
				Ok(hash) => {
					if let Some(expect) = &args.expect {
						if hash != *expect {
							eprintln!(
								"import: stored tree {}, but --expect named {} (the stored tree remains in the heap; removal is gc's job)",
								hash.as_hex(),
								expect.as_hex()
							);
							process::exit(exitcodes::HASH_MISMATCH);
						}
					}
					println!("{}", hash.as_hex());
					process::exit(exitcodes::SUCCESS);
				},
				Err(
					e @ import::ImportError::Refused {
						..
					},
				) => {
					eprintln!("import: {e}");
					process::exit(exitcodes::BAD_TAR);
				},
				Err(import::ImportError::Io(e)) => {
					eprintln!("import: {e}");
					process::exit(exitcodes::IO_ERROR);
				},
			}
		},
		cmds::Subcommands::Repair(_) => {
			require_repo(&repo, &root_args.repo);
			// Restores any subdirectory that's gone missing since the repo was made, same as add:
			// a vanished blobcas dir is exactly the kind of damage repair exists to rebuild from.
			if let Err(e) = repo.create() {
				eprintln!("{e}");
				process::exit(exitcodes::IO_ERROR);
			}
			match repair::repair(&repo) {
				Ok(report) => {
					eprintln!(
						"repair: verified {} blobs and {} trees; relinked {} files, donated {} blobs, normalized {} blob modes",
						report.blobs_verified,
						report.trees_verified,
						report.files_relinked,
						report.blobs_donated,
						report.blob_modes_normalized
					);
					if let Some(qdir) = &report.quarantine_dir {
						eprintln!(
							"repair: corruption found: quarantined {} blobs and {} trees into {}",
							report.blobs_quarantined,
							report.trees_quarantined,
							qdir.display()
						);
						process::exit(exitcodes::CORRUPTION_FOUND);
					}
					process::exit(exitcodes::SUCCESS);
				},
				Err(e) => {
					eprintln!("repair: {e}");
					process::exit(exitcodes::IO_ERROR);
				},
			}
		},
		cmds::Subcommands::Index(args) => {
			require_repo(&repo, &root_args.repo);
			// Restores any subdirectory that's gone missing since the repo was made, same as add:
			// that includes a treeidx dir that a heap from before indexes never had.
			if let Err(e) = repo.create() {
				eprintln!("{e}");
				process::exit(exitcodes::IO_ERROR);
			}
			let result = if args.all {
				index_backfill::index_all(&repo)
			} else {
				index_backfill::index_named(&repo, &args.hashes)
			};
			// Nothing goes on stdout: an index's path follows from its hash, so there's no result to hand a script,
			// and the exit code says whether every tree asked about now has one.
			match result {
				Ok(report) => {
					eprintln!(
						"index: indexed {} trees; {} already indexed; {} corrupt; {} not in this heap",
						report.indexed,
						report.already_indexed,
						report.corrupt.len(),
						report.not_in_stock.len()
					);
					for c in &report.corrupt {
						eprintln!(
							"index: tree {} is corrupt: its content hashes to {}, so no index was written for it (`gar repair` will move it into quarantine)",
							c.name.as_hex(),
							c.computed.as_hex()
						);
					}
					for hash in &report.not_in_stock {
						eprintln!("index: tree {} is not in this heap", hash.as_hex());
					}
					// When both happen, corruption decides the exit code:
					// it's damage in the heap that needs acting on, where an absent tree is only a request that missed.
					if !report.corrupt.is_empty() {
						process::exit(exitcodes::CORRUPTION_FOUND);
					}
					if !report.not_in_stock.is_empty() {
						process::exit(exitcodes::NOT_IN_STOCK);
					}
					process::exit(exitcodes::SUCCESS);
				},
				Err(e) => {
					eprintln!("index: {e}");
					process::exit(exitcodes::IO_ERROR);
				},
			}
		},
		cmds::Subcommands::Hash(_) => unreachable!("handled before repo search"),
	};
	// let r = repo::Repo::new("/tmp");
	// r.create_dir_all().expect("waa");
	// let hash = add::add(&r, "/tmp/slurpie", add::FaithMode::LinkOriginals).expect("whee");
	// println!("{hash:?}")
}

/// Reports and exits with `NO_REPO` when the repo isn't present on disk.
///
/// The message depends on how the repo was named: an explicit `--repo` can be echoed back,
/// while a failed search needs a hint about what was being searched for.
fn require_repo(repo: &repo::Repo, explicit_repo_arg: &Option<PathBuf>) {
	if repo.exists() {
		return;
	}
	match explicit_repo_arg {
		Some(_) => eprintln!("no gar repo at {:?}", repo.repo_path()),
		None => eprintln!("this subcommand needs to be run within, or pointed to, a gar repo"),
	}
	process::exit(exitcodes::NO_REPO);
}
