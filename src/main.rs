use std::fs;

mod add;
mod cmds;
mod gittree;
mod repo;

mod clap {
    pub use clap::error::ErrorKind;
    pub use clap::Parser;
}
use clap::Parser;

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
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    match root_args.subcommand {
        cmds::Subcommands::Init(_) => {
            println!("hello init");
        }
        cmds::Subcommands::Add(args) => {
            println!("hello add: {:?}", args.path);
        }
    };
    // let r = repo::Repo::new("/tmp");
    // r.create_dir_all().expect("waa");
    // let hash = add::add(&r, "/tmp/slurpie", add::FaithMode::LinkOriginals).expect("whee");
    // println!("{hash:?}")
}
