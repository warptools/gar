GAR -- a git-like file archive
------------------------------

Gar is for making snapshots of files and directories and storing them organized by their hash --
specifically, the same hash as a git tree hash.

Internally, Gar makes a series of hardlinks so that when the same files get stored repeatedly,
they're deduplicated and they don't cost you additional storage space.

This might be useful if you store a lot of similar or overlapping filesets repeatedly.
Common examples where this comes up are:

- storing snapshots of a set of files as they evolve over time.
- working with file sets produced by build tools or container processes.
- any other situation where you might want an append-only deduplicated cache of files.

Gar works fine with large files.

To see more about the filesystem layouts and the hashes used by gar,
see [./README_formats.md](./README_formats.md),
or the examples below.


Example Filesystem
------------------

Sometimes an example makes things clear fast, so let's start with that, and then explain it later ;)

If you run `gar init && gar add .` in a directory containing gar's own source code (for example!),
it'll think for a millisecond or two,
say "8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48"
(or some other hash like that),
and if you then do `find .gar/`,
you'll see it's created some directories and files with paths something like this:

```
...
.gar/blobcas/aa/d6/aad6b5d9f40cd90598235b9ff2bcfc8dbae1c8b391c5dd5b2c6231b22aa9c5db
.gar/blobcas/1f/4f/1f4fadf0177a322792c5e4ea099719b86dc09c156efc1c63588b27f1ee22020f
.gar/blobcas/24/15/2415e57ca7d0c6f54517f6fec92d66d673c1965c2b3817b136a8e2c504cc89d5
.gar/blobcas/a7/57/a7573b95a28adc55e8b33d49407202178b0008faf4ecc39495e2e8737883a4dd-x
.gar/blobcas/f2/25/f2250085e10ef5db59e425e673796108cf2c059a49e6f7451177cd2dd75ff0b3
.gar/blobcas/08/a6/08a6ae3fc51c74b826f1d7789f0736989db40d9a2f0800c0f0c24ff81e47d288
.gar/treecas/87/68/8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48/LICENSE-MIT
.gar/treecas/87/68/8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48/Cargo.toml
.gar/treecas/87/68/8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48/.gitignore
.gar/treecas/87/68/8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48/src
.gar/treecas/87/68/8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48/src/main.rs
...
```

The upshot is: if you look in `.gar/treecas/{ab}/{cd}/{thehash}`, you'll see a snapshot of the filesystem you added.
(The `{ab}/{cd}` part is just the first four characters of the hash: that keeps any one directory from getting enormous.)

And if you do that again on the same filesystem, it'll say the same thing,
and cost no more storage.

And if you do it again on a *similar* filesystem, it'll say a different hash,
and make a new "treecas" dir,
but _only files that changed_ will appear in the blobcas dir,
and the overall size growth on disk will be similarly minimal.

(Please do verify the size savings!  But beware that it's a little tricky :)
You can use the standard unix "`du`" command to ask about the size of these directories,
but be aware that the order of arguments will matter, and seperate `du` commands will report things differently than multiple arguments to the same commands!
his is because `du` only reports the size of a hardlinked file *the first time it sees it*... but multiple `du` commands of course are seeing things afresh.
So, `du` will probably say some interesting things, like claiming one of either `.gar/treecas` or `.gar/blobcas`
takes up nearly zero space on disk: that'll be because of the hardlinks.
But the exact results may vary based on your incantation.
And based on the size of directories on your filesystem.  And such fun things as that.)


Features
--------

Gar's main commands are `gar init` and `gar add`.

- `gar init` creates a "gar heap", which is a "`.gar/`" directory and few more directories with that.
- `gar add <path>` scans the given path, hashes all its contents, and stores a snapshot of it into the nearest gar heap it can find.
- `gar add <path>` returns the hash when it's done.  You'll then be able to find the snapshot of your files in `.gar/treecas/{ab}/{cd}/{hash}`.
- `gar path <hash>` prints the absolute path of that snapshot.
  (Scripts, use this!  The sharded layout is Gar's business; you shouldn't have to compute it yourself.)
- `gar gc --demands <file>` removes everything from the heap except the trees the demands files name.
  (See the garbage collection section below.)
- `gar export <hash>` writes that tree to stdout, as a tar stream.
- `gar import [--expect <hash>]` reads a tar stream from stdin, stores it like `add` does, and prints the treehash.
  (See the tar transport section below.)

A "gar heap" consists of the following two (or three) directory trees:

- blobcas -- a large, even-depth directory where every file is stored with a name that is its sha256 blobhash.
	- plus a suffix of "-x" for files that are executable (this is necessary because hardlinks share mode bits as well as contents).
- treecas -- a large, even-depth directory where every dir below the shard levels is a treehash and contains entirely that tree, materialized.
	- every file within these is a hardlink to a file in the blobcas.
	- not every subtree is materialized at the root of this dir (we don't have dir hardlinks, so there would be no point ;)).
- treeidx -- a directory where every file contains a compact, deterministic, serial representation of the expected hashes of every subtree in each materialized member of treecas.
	- this is not used in normal read operations (and its creation can be disabled entirely, or it can be regenerated later!).
	- Its role is to help corruption detection passes point more easily at specific subdirectories that are desynced with the overall tree hash, if corruption does occur.
	- another role is to support efficient read of whole trees if the Gar heap is exposed over a transport like plain HTTP.  The index file can be transfered in a single request, and thereafter contains enough information to identity every other fetch required to get every blpb, wit no dir walking required... and enough information to make progress bars possible, too!
	- (note: while Gar is heavily, heavily based on git, the format of these particular index files is not.  Gar has opted for a much simpler, more readable, and more deterministic format for these.)

Every time you some file and dirs to Gar, the files get copied into blobcas first,
and then dirs get made in the treecas, and then all their contents get filed in with hardlinks to the blobcas.

`gar add` *copies* data into the heap: once it returns, the heap shares no inodes with the files you gave it,
and the originals remain yours to edit, free of strings.
(A hardlink-the-originals mode exists in the library, for callers that can promise the originals will never be written again --
hardlinks are not copy-on-write, but truly a link to the same file,
so a write to an original in that mode would corrupt the gar heap.  The CLI deliberately does not expose it.)

Gar heaps are trustless structures.  They can be validated entirely from their contents.
For each file in the blobcas, its hash can be computed freshly from its contents.

### garbage collection

Gar heaps are append-only in normal operation; `gar gc` is how disk space comes back.

`gar gc --demands <file>` removes every tree not named by a demands file,
and then every blob no kept tree still links to.
A demands file names the treehashes to keep, one per line
(the exact format is specified in [./README_formats.md](./README_formats.md)).
The flag can be repeated, and the kept set is the union of all the files.
Demanding nothing is legal but must be said explicitly, with an empty file: a bare `gar gc` is refused.

Gc fails closed: a demands file that doesn't parse means nothing is deleted at all,
and a demanded tree that isn't in the heap is a failing exit code
(though the gc still completes for everything else first).

Gc must not run concurrently with any other use of the heap, reads included:
an add may hardlink to a blob the sweep is about to judge unreferenced,
and anyone already walking a tree the gc removes will have it deleted out from under them.
Nothing locks against this yet, so it's the caller's job to schedule gc in a quiet moment.

### transport with tar

Moving a tree between heaps is a pipe:

```
gar export $hash | ssh elsewhere gar import --expect $hash
```

Export is deterministic: the same tree is the same bytes, every time, on every machine.
The stream is plain, uncompressed tar, readable by ordinary tar tools --
and pipes compose, so `gar export $hash | zstd` is the compression story.
Empty directories ride along (they're part of the tree hash, so they must).

Import is the other end, and it's trustless and fail-closed.
The printed treehash is computed from what was actually stored, never taken from the sender's word,
so `--expect` plus the exit code is a complete integrity check.
A mismatched `--expect` is a failing exit, but the stored tree legitimately stays in the heap
under the hash it really has: a heap is append-only, and removal is gc's job.
Streams that misbehave are refused outright:
entry types other than files, directories, and symlinks (devices, fifos, hardlink entries),
and paths that are absolute, try to `..` their way out of the import,
or try to escape by writing through a symlink.

Import doesn't care who made the tar.  Feeding it archives made by other tools works,
within the entry-type rules above -- which makes it a way to unpack many overlapping
tarballs without spending disk on what they share.

The exact stream format is specified in [./README_formats.md](./README_formats.md).

### Supported Filesystem Attributes and Features

#### symlinks

Yes.  Symlinks are supported and filesets containing symlinks can be stored in Gar with without issue.

#### empty directories

Yes.  Gar stores and recreates empty directories, and they contribute to the tree hash.

(This is Gar's one intentional divergence from git's behaviors.)

Gar still emits a well-formed git tree object when it does this.
The entry is mode "40000" pointing at the hash of the empty tree --
exactly the object `git mktree` produces if you hand it such an entry directly,
and `git ls-tree` reads the result back without complaint.

(If you're interested in deep git lore: the empty dir limitation
in git resides in their index format, not in the object format.
Strange but true.)

The practical consequence is low unless you're comparing hashes across the two tools:
`gar hash` and `git write-tree` agree exactly on the same directory,
right up until that directory contains an empty directory somewhere.
Then, Gar does something I'd argue is "more correct", but the hash will be different.

#### posix permissions

Gar does exactly what git does: it ignores every part of posix permissions, except the "x" (execute) bit.

Files and directories are always readable and writable (according to their posix bits).
This is to the match the expectations that people have from working with git.

#### uids and gids

Gar does not store distinct uids and gids.

Reasons for this are the same as with other posix modes and permissions --
uids and gids are not included in a git treehash, so they're not what Gar aims to do;
and also, managing them would actively obstruct blob dedup --
and on top of all that, it would just plain mean Gar needs to be much more complex so that it can support running with elevated privileges, with all the tricky, subtle code that implies.
So, no.
Each Gar heap contains exactly one uid and exactly one gid,
and it's whatever your current running uid and gid are.

It is acceptable to create different gar heaps with different uids and gids,
but it is unlikely to be a good idea to try to share the same gar heap directory between a mixture of uids and gids.

#### other metadata

Gar does not store file modification times or other such time information.

Remember, Gar is exactly as per git _tree hashes_: and tree hashes don't include mtime.
(Commit hashes do.  But those aren't what Gar is based on;
and they're not helpful for content-addressed dedup, which is Gar's main purpose.)
So we can't have times mucking things up.

(Hardlinks also again contribute to this decision.  See previous section.
Tl;dr: we'd be forced to do more actual data copying in order to support diverse mtimes, too -- yikes; no thanks.)

Other attributes like ctime and atime are not considered by Gar at all.
(Ctime in particular is typically fundamentally unsettable short of writing a filesystem driver so,
to ignore this is... Let's just say it's a pretty normal choice.)


Using Effectively
-----------------

You can use Gar however you want,
but one particularly neat option to consider is...
use read-only bind mounts to put the treecas content where you want it, at whatever path and dir name you want it as.
This is fast, cheap, doesn't require escalated privileges on most modern linuxes,
and most importantly, provides an excellent barrier again accidental mutation of the CAS --
which is great if working with code that isn't fully trusted to Play Nice, or simply as defense-in-depth against accidental sanity excursions.


Comparisons
------------

### comparison to git

In common:
Git and Gar both use content-addressing to identify snapshots of data with a hash.
In fact, they use exactly the same hashing for this -- Gar copies what Git does.

Different:
Git stores "commits", which have an ordered, historical relationship and metadata about time and authors,
in addition to storing files and directories (which it calls "trees").
Gar does not have any concept of versions.  Gar only stores what git calls "trees".
Gar also stores empty directories, which git's index cannot;
see the section on empty directories above for what that means for hash comparisons.



Future (possible) Features
--------------------------

### labels

It might be useful to add another optional directory to Gar heaps
which contains human-readable labels that are mapped to a treecas hash.
(This would be reminescent of how git branches and tags point to commit hashes.)

### treeidx files

Treeidx files are meant to contain an all-in-one list of paths in a treecas entry, along with their sizes.
The purpose of this would be to let a "dumb" HTTP client looking at a gar heap presented by a "dumb" HTTP server with no special configuration
be able to download all contents of a tree, knowing only the treehash
(and offer a decent progress bar while doing it, too).

Transport based on treeidx files would involve more connections than a single tar stream (depending on http protocol version),
but would also allow better deduplication and transfer of only needed subsets of data if the gar heap doing the retrieval already has some contents.

See more about the format of treeidx files in [./README_formats.md](./README_formats.md).



Contributing
------------

It's a pretty standard rust/cargo workspace.

The code is split across three crates:
`gittree` (git-compatible tree hashing, standalone),
`gar` (the repo and add logic, built on `gittree`),
and `gar-cli` (the `gar` binary).
A fourth crate, `experiments`, holds benchmarks and is not product code.

Run `make setup` once after cloning: it activates the git hooks.
Run `make` to see all commands.
Before committing anything, run `make all` (fmt, build, test).
There should be no warning messages.


License
-------

We use a dual license of Apache2 or the MIT license, at your option.
This is intended to be a freedom-maximizing choice.
If you submit code to this repo, you agree your contributions will be redistributable under these licenses.

SPDX-License-Identifier: Apache-2.0 OR MIT
