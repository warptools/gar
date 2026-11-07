Formats
--------

In this file, we cover the formats used by Gar in detail.

That includes: {what gets hashed} and {how data is laid out on disk}.


### high level: most of this is still effectively git

Most of Gar is exactly as per git.

Blob hashes in Gar are git blob hashes (using sha256).

Tree hashes in Gar are git tree hashes (using sha256).

The few exceptions to Gar's data model similarity to Git are minor:
Gar does not support git submodules (probably unsurprising);
and Gar *does* store empty directories (useful, but potentially surprising,
depending on the strength of your stockholm syndrome about that particular Git misfeature).
(Empty directories are even handled in a way Git itself understands, just does not do!)

However, the file and directory layouts are considerably different.
Gar prioritizes determinism (and concurrent read and writability),
and also keeps all files unpacked for hardlinking,
and this leads to significantly different conclusions than Git's object dir and packfiles.


### sharding

The blobcas, the treecas, and the treeidx directory are all sharded:
entries don't sit directly in the top directory, but two directory levels below it.
The first level is named by the first two characters of the hash, in hex;
the second level by the next two;
and the entry itself is named by the *full* hash, not the remainder.
Hex is always lowercase.

```
.gar/blobcas/29/09/2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce
.gar/blobcas/29/09/2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce-x
.gar/treecas/21/28/2128f977414c236e7a4367508fbc095549e50ae05f26983540e86b0abd7d1e58/
.gar/treeidx/21/28/2128f977414c236e7a4367508fbc095549e50ae05f26983540e86b0abd7d1e58
```

Shard directories are created when the first entry is stored in them, not in advance.
A reader should treat a missing shard directory the same as a missing entry.


### the blobcas directory

The blobcas directory is full of files with names that are the hex encoding
of the git blob hash using sha256, sharded as described above.

The git blob hash is distinct from the sha256 hash of the file itself.
More specifically, the hash has a preamble of the word "blob" and then the size of the following body.
More details can be found in the git specification.

As a special further rule, files with the executable bit set are given an "-x" suffix to their name in the blobcas.
This is because hardlinks do not permit storing different metadata attached to paths that hardlink to the same data;
so, we have to store content separately when it has distinct metadata.

Blobs are stored read-only: mode `0444`, or `0555` for the executable ("-x") blobs.
This is a format guarantee: a blobcas inode is never written to (nor re-moded) after it is published,
and the read-only modes protect it from accidental mutation through any of its hardlinks --
including the ones in the treecas.
(Git's object files and the nix store make the same choice.)
Removing a blob is still governed by the permissions of the directory holding it, so this costs nothing at cleanup time.

(One caveat: heaps populated via the library's link-originals mode share inodes with the user's original files,
mode bits included, so the read-only guarantee holds only for heaps populated by copying -- which is what the CLI does.)


### the treecas directory

The treecas directory is full of directories with names that are the hex encoding
of the git tree hash using sha256, sharded as described above.

(If you're not familiar with the git treehash: it's similar to the commit hash,
but with slightly *less* data: the treehash covers _only the files and directories_ in the commit.
A git commit hash is composed over the time of the commit, the author information, the commit history links, and then the treehash.
Gar does not include commit history relationships, and so uses only the treehash.)

Each directory is a snapshot of some data that was given to `gar add`.
Every file in each directory is a hardlink to files in the blobcas directory,
which is how Gar deduplicates disk space usage.


### the treeidx directory

The treeidx directory is full of files with names that are the hex encoding
of the git tree hash using sha256, sharded as described above.

Each one is the treeidx file (see the format below) of the treecas entry with the same name.
`gar add` writes one for every tree it stores, `gar fetch` for every tree it fetches, and `gar index` for any tree lacking one.
They're derived data: any of them can be regenerated from its tree, and nothing reading a local heap needs them.
They're what lets a heap be read from afar, through nothing smarter than a static webserver.

An index file can outlive its tree, and that's not corruption:
`gar gc` keeps an index exactly when its treehash is demanded, whether or not that tree is in stock,
and `gar repair` leaves index files alone when it quarantines a tree.


### the quarantine directory

A `quarantine` directory appears beside the blobcas and treecas only when `gar repair` has found corruption:
entries whose content provably doesn't match their name are moved here, never deleted.

Each repair run that quarantines anything makes one subdirectory, named `<unix-seconds>-<pid>`,
and inside it the condemned entries keep their sharded paths under `blobcas/` or `treecas/`
(so `.gar/quarantine/1767800000-4242/treecas/87/68/8768db…` is where a tree named `8768db…` would land).

Nothing in gar reads the quarantine: it's out of circulation, held for human forensics.
Quarantined trees still hold hardlinks to blobcas inodes, so their disk space
is not reclaimed -- not even by `gar gc` -- until the quarantine directory itself is deleted,
which is a decision gar leaves to you.


### demands files

Demands files are how other tools tell `gar gc` what to keep:
each one names the treehashes that must stay in stock, and `gar gc` removes everything else.
They're an input format: gar reads them, never writes them, and they don't live inside the heap.

```
# produced by (your tool here)
8768db56830498631de8cedd4f40686696953766fefc0f73e28680c998936e48
2128f977414c236e7a4367508fbc095549e50ae05f26983540e86b0abd7d1e58
```

The format:

- The file is plain text, split into lines by the LF character (0x0A).
  (CRLF is not tolerated: a demands file drives deletion, so a mangled file must fail to parse rather than be guessed at.)
- Each line is exactly one of three things:
	- a treehash: exactly 64 lowercase hex characters, and nothing else on the line;
	- a comment: the `#` character as the first character of the line, and then anything;
	- or blank: completely empty.
- Anything else is a parse error, and a parse error anywhere rejects the whole file
  (and a gc handed a rejected file deletes nothing at all).

Duplicate hashes are allowed and mean nothing extra.
An empty file (or one that's all comments) is legal: it demands nothing.


### the tar transport stream

`gar export` writes a treecas entry as a tar stream; `gar import` reads one back into a heap.

What export writes is normative, and deterministic down to the byte:

- Entries appear in the git-wise walk order (the same order tree hashing visits them:
  byte order of names, directories compared as if suffixed with `/`).
- Headers are in the GNU tar format.  Paths too long for the header field use GNU longname entries;
  pax extensions are never written.
- Paths are bare relative names: no leading `/`, no `./` prefix, no `..`.
  The tree's root directory is the stream itself and gets no entry; directory names end with `/`.
- Every header field that could vary is pinned: mtime 0, uid/gid 0, empty uname/gname.
- Modes are exactly the heap's guarantees: `0444` for files, `0555` for executable files,
  `0755` for directories, and `0777` for symlinks (whose mode bits mean nothing; that's the tar convention).
- Empty directories appear as directory entries, like any other directory.
- No compression.  Compose pipes for that.

What import accepts is normative too, and fail-closed:

- Entry types: files, directories, and symlinks.  Anything else (hardlink entries, devices,
  fifos, pax specials) is refused, by entry name, storing nothing.
- Paths must stay inside the import: absolute paths and paths with `..` components are refused,
  and so is writing through a symlink carried earlier in the stream.
  `./` spellings are tolerated (common in foreign tars) but never required.
- Modes are normalized on landing, exactly as `gar add` would:
  only the user-exec bit of a file entry is read; everything else is discarded.
  Likewise uid/gid/mtime: not part of a tree, not stored (see the metadata sections above).
- The treehash import prints is computed from the stored bytes.
  A sender's claims are checked by arithmetic, never believed.


### treeidx files

A treeidx file lists every path in one tree, with everything needed to recompute the tree's hash by arithmetic alone,
plus the size of every file.

Treeidx files are sort of comparable to git pack index files in role, but Gar opted for a much simpler format for these files.
(The git format is binary, and very, [very complex to parse](https://git-scm.com/docs/pack-format) as well as generate,
and much of that complexity isn't relevant to the subset git that Gar provides equivalences with,
so overall attempting to directly reuse that format would provide little to no value, at extreme cost.)

Treeidx files are meant to be relatively human-readable (and even diff'able).

They're linebreak delimited[^1], with each line containing one entry, and each entry describing one path (a file, a directory, or a symlink), looking like this:

```
treeidx v1
    2 ./ 040000 - 9024a7f8afa43db06ff2b50d9ac9c21b791bee49d8092d3f14f1e433bfd927fa
    8 ./a_dir/ 040000 - e1896fb25dd721b447c52e40267a90405ebc41aaa2c7143e9cf58cf5c8421cde
   15 ./a_dir/deeper/ 040000 - 9897054d9f01c666ac1371d3e0a022a67b5df59ddb1608e8165a3b1fa22da706
   23 ./a_dir/deeper/samefile 100644 10 4698ba4d7c51602d6a50e4fb6e150e2e06d625ba5874cde627bc6dfc357a23db
   18 ./a_dir/more_files 100644 10 4698ba4d7c51602d6a50e4fb6e150e2e06d625ba5874cde627bc6dfc357a23db
   18 ./a_dir/other_file 100644 12 8431d03990244d0bffa3dfecdd7a67d0bca2f5e999bff04469cde93cc2365d96
    8 ./a_file 100644 7 2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce
   11 ./a_symlink 120000 13 target string
```

(Note the symlink at the end: its target has a space in it, and it's the 13 that says where the target ends.)

The parse of this is as follows:

- (The first line is a version and filetype indicator.  Currently this is always exactly "treeidx v1".)
- Each entry starts with a fixed width five bytes which contains an ascii base-10 number, right-aligned by prefix padding with space characters, followed by a space character.
- Next is the path: this has the length (in bytes) indicated by the preceding number.  Another space character follows the path.
	- Note!  You _must_ use the length prefix to determine how long the path is, in order to correctly handle paths that contain spaces or linebreaks characters!
	- The length counts the path exactly as written: its `./` prefix, and a directory's trailing `/`, included.  So the longest path a treeidx can carry is 99999 bytes, decoration and all[^2].
- The git tree mode number follows.  (This is six bytes, and effectively is one of four values: "100644" for files, "100755" for executable files, "040000" for dirs, and "120000" for symlinks.)  Then, another space.
- A size hint follows, as an ascii base-10 number.  For directories, it is instead a dash character.  Then, another space.
	- For a file, it's the file's length in bytes.  For a symlink, it's the length in bytes of the symlink's target.  Either way, it must be exact.  (Why it's still called a hint, and why it can be trusted: see below.)
- The treehash (or blobhash) sha256 of the object, encoded in lowercase hex (all 64 characters), follows.
	- Except for symlinks, which have no hash here: their target follows instead, as raw bytes, exactly as many as the size hint says.  (A symlink's blobhash is just the hash of its target, so anyone holding the target can compute it.)
- Then, a line break.  That's the end of the entry.

Every tree has exactly one valid treeidx, down to the byte, and a parser refuses anything else, whole:

- The first entry is the root directory, spelled `./`.
  After it, entries come in depth-first preorder: a directory's entry is followed by everything inside it, before whatever comes after that directory.
- Entries within one directory are in git's tree order: byte order of names, with directories compared as if suffixed with `/`
  (the order tree hashing visits them in, and the order `gar export` writes).
- Numbers are plain ascii decimal, with no leading zeros.
- Outside of paths and symlink targets, spaces appear only as single separators between fields, and as the length field's left padding.
- Paths begin with `./`, and end with `/` if and only if they're directories.
  In between, every `/`-separated segment is non-empty, is never `.` or `..`, and never contains a NUL byte.
- No path appears twice -- not even once as a file and once as a directory.
- Symlink targets are non-empty and never contain a NUL byte.  (No symlink on disk can have either.)
- The file ends right after the last entry's line break.

A treeidx is a list of claims, and every one of them can be checked.
Gar checks them all before committing anything an index describes:

- Hashes: everything git's tree hashing needs is right there in the entries -- names, modes, blob hashes, and symlink targets (which hash inline) --
  so every directory's hash, the root's included, recomputes from the entries by pure arithmetic, with no file IO.
  A directory whose claimed hash doesn't recompute, or a root that isn't the treehash asked for, gets the whole index refused,
  before a single blob is fetched.
- Sizes: tree hashes don't cover sizes (which is why the field is a "hint"), so the arithmetic can't check them.
  But a blob's hash commits to its length, so each size claim can be checked against the blob it names, wherever that blob came from.
  `gar fetch` checks every one, even for blobs it already had and so didn't fetch,
  and it stops reading any blob that runs past its claimed size.

And gar never stores a treeidx it was handed: it writes its own, canonically, from the entries it checked.
So a lie in a remote's index can't survive into a heap, and every size hint inside a heap is exact.

Some incidental details to note:

- Paths always begin with a dot.  This is redundant, but we consider it a visual nicety.
- Paths that are directories are encoded with a trailing slash.  This is redundant considering the mode number also indicates directories, but we consider it a visual nicety.
- The choice of a fixed with length indicator is for (yes, again -- you guessed it) visual nicety.  It makes it easier for a human reader to scroll through a large treeidx file and see shared path prefixes by just eyeballing it.
- If you're _sure_ there's no spaces or linebreaks in any filenames (or symlink targets) in a dataset you're examining... yes, you can parse this easily with `awk`  or even sheer `column` in a shell script. ;)


[^1] -- Sort of.  The treeidx format uses linebreaks, and the linebreaks are required, but a parser should not _parse_ by splitting upon them.  In the event of a filename (or symlink target) that contains a linebreak (unusual, but legal!), there will be _another line break_ in the treeidx file.  The only fully correct way to parse is by reading the length numbers at the start of each entry (and, for symlinks, the size hint).

[^2] -- About path size limits: we've decided that "five digits base 10 is enough", but if you truly dive into this, one can argue that there are not rules.  Path limits could come from the kernel, or from the filesystem.  When they're from the kernel, they can be changed by... recompiling the kernel.  Some numbers we've seen in the wild are: NTFS specified a length limit of 32768 bytes for a whole path.  Linux typically specifies 256 bytes per path segment, and reputedly at some point had a limit of 4096 overall (although I currently find no evidence of a limit this low on my systems).  Windows has been known to specify some truly short numbers we will not discuss.  But ultimately: when is the last time you handled a path more than 4 kilobytes long?  We think Gar can be useful even when enforcing a limit some twenty-four times that.
