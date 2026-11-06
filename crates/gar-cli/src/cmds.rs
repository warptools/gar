use std::path::PathBuf;

#[derive(clap::Parser, Debug)]
pub struct Root {
	#[command(subcommand)]
	pub subcommand: Subcommands,

	/// Path to the Gar repo to operate on.
	/// This names the repo exactly: nothing is searched for above it.
	/// When not provided, the repo is detected by searching up from the cwd.
	///
	/// Gar repos are always indicated by a ".gar" directory.
	#[arg(long)]
	pub repo: Option<PathBuf>,

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

	/// compute the git tree hash of a path, without touching any Gar repo or storage.
	Hash(HashCmd),

	/// print the absolute path of a tree already in Gar storage.
	Path(PathCmd),
}

#[derive(clap::Args, Debug)]
pub struct InitCmd {}

#[derive(clap::Args, Debug)]
pub struct AddCmd {
	/// path to the directory to add to Gar's storage.
	pub path: PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct HashCmd {
	/// path to the file, directory, or symlink to hash.
	pub path: PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct PathCmd {
	/// treehash to resolve: exactly 64 lowercase hex characters.
	/// Uppercase is rejected, not downcased: gar itself only ever prints lowercase.
	#[arg(value_parser = parse_treehash)]
	pub hash: gittree::Hash,
}

/// Parses a treehash argument strictly: exactly 64 lowercase hex characters.
///
/// `Hash::from_hex` alone would also accept uppercase; the CLI rejects that instead of downcasing,
/// since every hash gar prints is lowercase, and anything else suggests a mangled value.
fn parse_treehash(s: &str) -> Result<gittree::Hash, String> {
	if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
		return Err("a treehash is exactly 64 lowercase hex characters".into());
	}
	Ok(gittree::Hash::from_hex(s).expect("just validated as 64 hex chars"))
}
