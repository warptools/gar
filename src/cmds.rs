use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
pub struct Root {
    #[command(subcommand)]
    pub subcommand: Subcommands,

    /// Raise verbosity by specifying this flag repeatedly.
    #[arg(short='v', long, action = clap::ArgAction::Count)]
    pub verbosity: u8,
}

#[derive(clap::Subcommand, Debug)]
pub enum Subcommands {
    /// initialize a new empty Gar repo.
    Init(InitCmd),

    /// add local files and directories to Gar storage.
    Add(AddCmd),
}

#[derive(clap::Args, Debug)]
pub struct InitCmd {}

#[derive(clap::Args, Debug)]
pub struct AddCmd {
    /// path to the directory to add to Gar's storage.
    pub path: PathBuf,
}
