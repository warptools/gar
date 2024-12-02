Formats
--------

Most of Gar is exactly as per git.

Blob hashes in Gar are git blob hashes (using sha256).

Tree hashes in Gar are git tree hashes (using sha256).


### the blobcas directory

The blobcas directory is full of files with names that are the hex encoding
of the git blob hash using sha256.

The git blob hash is distinct from the sha256 hash of the file itself.
More specifically, the hash has a preamble of the word "blob" and then the size of the following body.
More details can be found in the git specification.

As a special further rule, files with the executable bit set are given an "-x" suffix to their name in the blobcas.
This is because hardlinks do not permit storing different metadata attached to paths that hardlink to the same data;
so, we have to store content separately when it has distinct metadata.


### the treecas directory

The treecas directory is full of directories with names that are the hex encoding
of the git tree hash using sha256.

(If you're not familiar with the git treehash: it's similar to the commit hash,
but with slightly *less* data: the treehash covers _only the files and directories_ in the commit.
A git commit hash is composed over the time of the commit, the author information, the commit history links, and then the treehash.
Gar does not include commit history relationships, and so uses only the treehash.)

Each directory is a snapshot of some data that was given to `gar add`.
Every file in each directory is a hardlink to files in the blobcas directory,
which is how Gar deduplicates disk space usage.


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
