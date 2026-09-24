mod cmds;
mod paths;

use gar::add;
use gar::repo;

mod clap {
	pub use clap::Parser;
	pub use clap::error::ErrorKind;
}
use clap::Parser;

use std::process;

fn main() {
	let root_args = match cmds::Root::try_parse_from(std::env::args_os()) {
		Ok(args) => args,
		Err(e)
			if matches!(
				e.kind(),
				clap::ErrorKind::DisplayHelp | clap::ErrorKind::DisplayVersion
			) =>
		{
			println!("{e}");
			process::exit(0);
		},
		Err(e) => {
			eprintln!("{e}");
			process::exit(1);
		},
	};

	// The hash subcommand is repo-independent; handle it before any repo search.
	if let cmds::Subcommands::Hash(args) = &root_args.subcommand {
		match gittree::hash_of_path(&args.path) {
			Ok(hash) => {
				println!("{}", hash.as_hex());
				process::exit(0);
			},
			Err(e) => {
				eprintln!("{}", e);
				process::exit(5);
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
				process::exit(5);
			},
		},
	};
	let repo = repo::Repo::new(&repo_dir);

	match root_args.subcommand {
		cmds::Subcommands::Init(_) => {
			if repo.exists() {
				println!("gar repo already exists at {:?}", repo.repo_path());
				process::exit(4);
			}
			match repo::init_at(&repo_dir) {
				Ok(r) => println!("gar repo created at {:?}", r.repo_path()),
				Err(e) => {
					eprintln!("{e}");
					process::exit(5);
				},
			}
			process::exit(0);
		},
		cmds::Subcommands::Add(args) => {
			if !repo.exists() {
				match &root_args.repo {
					Some(_) => eprintln!("no gar repo at {:?}", repo.repo_path()),
					None => {
						eprintln!(
							"this subcommand needs to be run within, or pointed to, a gar repo"
						)
					},
				}
				process::exit(3);
			}
			// Restores any subdirectory that's gone missing since the repo was made.
			if let Err(e) = repo.create() {
				eprintln!("{e}");
				process::exit(5);
			}
			// TODO: the default should become `Copy`, once that mode is implemented.
			//  See the large comment block in add.rs about this.
			match add::add(&repo, args.path, add::FaithMode::LinkOriginals) {
				Ok(hash) => {
					println!("{}", hash.as_hex());
					process::exit(0);
				},
				Err(e) => {
					eprintln!("{}", e);
					process::exit(5);
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
