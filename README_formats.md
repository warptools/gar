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

Both the blobcas and the treecas are sharded:
entries don't sit directly in the top directory, but two directory levels below it.
The first level is named by the first two characters of the hash, in hex;
the second level by the next two;
and the entry itself is named by the *full* hash, not the remainder.
Hex is always lowercase.

```
.gar/blobcas/29/09/2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce
.gar/blobcas/29/09/2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce-x
.gar/treecas/21/28/2128f977414c236e7a4367508fbc095549e50ae05f26983540e86b0abd7d1e58/
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


### garidx files

(This section is a draft.  Garidx files are not yet implemented.)

Garidx files are sort of comparable to git pack index files in role, but Gar opted for a much simpler format for these files.
(The git format is binary, and very, [very complex to parse](https://git-scm.com/docs/pack-format) as well as generate,
and much of that complexity isn't relevant to the subset git that Gar provides equivalences with,
so overall attempting to directly reuse that format would provide little to no value, at extreme cost.)

Garidx files are meant to be relatively human-readable (and even diff'able).

They're linebreak delimited[^1], with each line containing one entry, and each entry describing one path (a file or a directory), looking roughly like this:

```
# garidx v1
     2 ./ 040000 - afe3845ba76ec209f86
     5 ./foo 100644 99 9c4fba7ef811632cde
     7 ./some/ 040000 - b2a7ee7fc6ba1f387634
    16 ./some/script.sh 100755 4325 fe9fbc1da76325ef8e
```

The parse of this is as follows:

- (The first line is a version and filetype indicator.  Currently this is always exact "garidx v1".)
- Each entry starts with a fixed width five bytes which contains an ascii base-10 number, right-aligned by prefix padding with space characters, followed by a space character.
- Next is the path: this has the length (in bytes) indicated by the preceding number.  Another space character follows the path.
	- Note!  You _must_ use the length prefix to determine how long the path is, in order to correctly handle paths that contain spaces or linebreaks characters!
- The git tree mode number follows.  (This is six bytes, and effectively is one of four values: "100644" for files, "100755" for executable files, "040000" for dirs, and "120000" for symlinks.)  Then, another space.
- A size hint follows, as an ascii base-10 number.  For directories, it is instead a dash character.  Then, another space.
- The treehash (or blobhash) sha256 of the object, encoded in base58, follows.
- Then, a line break.  That's the end of the entry.

Some incidental details to note:

- Paths always begin with a dot.  This is redundant, but we consider it a visual nicety.
- Paths that are directories are encoded with a trailing slash.  This is redundant considering the mode number also indicates directories, but we consider it a visual nicety.
- The choice of a fixed with length indicator is for (yes, again -- you guessed it) visual nicety.  It makes it easier for a human reader to scroll through a large garidx file and see shared path prefixes by just eyeballing it.
- If you're _sure_ there's no spaces or linebreaks in any filenames in a dataset you're examining... yes, you can parse this easily with `awk`  or even sheer `column` in a shell script. ;)


[^1] -- Sort of.  The garidx format uses linebreaks, and the linebreaks are required, but a parser should not _parse_ by splitting upon them.  In the event of a filename that contains a linebreak (unusual, but legal!), there will be _another line break_ in the garidx file.  The only fully correct way to parse is by reading the length numbers at the start of each entry.

[^2] -- About path size limits: we've decided that "five digits base 10 is enough", but if you truly dive into this, one can argue that there are not rules.  Path limits could come from the kernel, or from the filesystem.  When they're from the kernel, they can be changed by... recompiling the kernel.  Some numbers we've seen in the wild are: NTFS specified a length limit of 32768 bytes for a whole path.  Linux typically specifies 256 bytes per path segment, and reputedly at some point had a limit of 4096 overall (although I currently find no evidence of a limit this low on my systems).  Windows has been known to specify some truly short numbers we will not discuss.  But ultimately: when is the last time you handled a path more than 4 kilobytes long?  We think Gar can be useful even when enforcing a limit slightly over twice that.
