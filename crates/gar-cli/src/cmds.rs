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

	/// remove everything from Gar storage except the trees named by demands files.
	Gc(GcCmd),

	/// write a tree already in Gar storage to stdout, as a tar stream.
	Export(ExportCmd),

	/// read a tar stream from stdin into Gar storage, and print the treehash it stored as.
	Import(ImportCmd),
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
pub struct GcCmd {
	/// path to a demands file: the treehashes to keep, one per line.
	/// Repeatable; the kept set is the union of all the files.
	///
	/// At least one file is required: a gc that's meant to keep nothing has to say so
	/// with an explicitly empty file, because a bare "gc" deleting everything is a footgun.
	#[arg(long = "demands", required = true)]
	pub demands: Vec<PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct PathCmd {
	/// treehash to resolve: exactly 64 lowercase hex characters.
	/// Uppercase is rejected, not downcased: gar itself only ever prints lowercase.
	#[arg(value_parser = parse_treehash)]
	pub hash: gittree::Hash,
}

#[derive(clap::Args, Debug)]
pub struct ExportCmd {
	/// treehash to export: exactly 64 lowercase hex characters.
	#[arg(value_parser = parse_treehash)]
	pub hash: gittree::Hash,
}

#[derive(clap::Args, Debug)]
pub struct ImportCmd {
	/// treehash the stream is expected to store as.  A mismatch is a failing exit,
	/// but the stored tree remains in the heap under the hash it actually has:
	/// a Gar heap is append-only, and removal is gc's job.
	#[arg(long, value_parser = parse_treehash)]
	pub expect: Option<gittree::Hash>,
}

/// Parses a treehash argument strictly, per `Hash::from_hex_strict`:
/// exactly 64 lowercase hex characters, with uppercase rejected rather than downcased.
fn parse_treehash(s: &str) -> Result<gittree::Hash, String> {
	gittree::Hash::from_hex_strict(s)
		.ok_or_else(|| "a treehash is exactly 64 lowercase hex characters".into())
}
