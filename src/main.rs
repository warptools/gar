mod add;
mod cmds;
mod gittree;
mod repo;

mod clap {
    pub use clap::error::ErrorKind;
    pub use clap::Parser;
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
        }
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };
    let repo_search_start = match root_args.repo {
        Some(path) => path,
        None => std::env::current_dir().expect("must be able to find cwd"),
    };
    let repo = repo::find_repo_from(&repo_search_start)
        .expect("io error while searching for existing repos");

    match root_args.subcommand {
        cmds::Subcommands::Init(_) => match repo {
            Some(repo) => {
                println!("warning: a repo already exists in this dir or its parent");
                println!("gar repo exists at {:?}", repo.repo_path());
                process::exit(4);
            }
            None => {
                let r = repo::Repo::new(repo_search_start).expect("opening repo");
                r.create_dir_all().expect("creating repo dirs");
                println!("gar repo created at {:?}", r.repo_path());
                process::exit(0);
            }
        },
        cmds::Subcommands::Add(args) => match repo {
            Some(repo) => {
                repo.create_dir_all().expect("creating repo dirs");
                match add::add(&repo, args.path, add::FaithMode::LinkOriginals) {
                    Ok(hash) => {
                        println!("{}", hash.as_hex());
                        process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        process::exit(5);
                    }
                }
            }
            None => {
                eprintln!("this subcommand needs to be run within, or pointed to, a gar repo");
                process::exit(3);
            }
        },
    };
    // let r = repo::Repo::new("/tmp");
    // r.create_dir_all().expect("waa");
    // let hash = add::add(&r, "/tmp/slurpie", add::FaithMode::LinkOriginals).expect("whee");
    // println!("{hash:?}")
}
