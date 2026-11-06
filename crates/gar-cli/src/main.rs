mod cmds;
mod exitcodes;
mod paths;

use gar::add;
use gar::demands;
use gar::gc;
use gar::repo;

mod clap {
	pub use clap::Parser;
	pub use clap::error::ErrorKind;
}
use clap::Parser;

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
						"gc: kept {} trees; removed {} trees and {} blobs",
						report.trees_kept, report.trees_removed, report.blobs_removed
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
