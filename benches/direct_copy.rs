/*
The purpose of this file is to probe into if it makes any difference
to try to use `fs::copy` (so that it can use `unix::kernel_copy` features)
vs doing fairly dumb shuttling of IO through userspace.

The answer appears to be "no".

On a tmpfs:

The fscopy and iocopy functions on both 10k and 10M are within a margin of equal.
The fscopy is fractions of a millisecond *slower* sometimes.
So, it seems clear that the `unix::kernel_copy` features are not usefully privileged.

When adding hashing into it (which necessitates an extra read, if using fs::copy):
copy_then_hash fares slightly worse than tee_and_hash, in both smaller and bigger files.
(I've also tested with 100M, not committed here, and it's similar.)
The difference is as high as about 20% on smaller files, and gets down to 5% on large files.

(The exact numbers are considerably erratic, even when criterion reports doing 100 samples.)

On ZFS:
on 10k: copy_then_hash is still minutely slower, as above on tmpfs.
on 10M: 15.5/18.5 = 16% slower to tee_and_hash.
on 100M: 160.2/187.4 = 15% slower to tee_and_hash.
So here the native copy *does* start getting bonuses, evidently.
On the other hand, I don't see similarly significant preference in the fscopy vs iocopy measurements,
so I really don't know what to think of that.
This is either a mild result, or possibly still simply sampling error.

On ext4:
... results are so wildly unstable that I don't know what to conclude
other than ext4 is a somewhat silly and inherently unpredictable filesystem.
Results swing back and forth by 30 to 50%, positive and negative,
so it's utterly unclear which approach could be said to be winning.

Overall:

It seems like things are dang near a toss-up, across the board.
The variances I do see seem minor and frankly make so little sense that I'm
not convinced the measurements aren't somehow specific to errata that's not worth coding around.

All of this could also be reviewed in contrast with rustix functions, if we add that.
I haven't studied it yet, but it's possible that crate would offer different IO abstraction levels.

*/

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::io;
use std::{fs, io::Write, path::PathBuf};

fn pth(filename: &str) -> PathBuf {
    PathBuf::from("/tmp/gar-test/").join(filename)
}

pub fn criterion_benchmark(c: &mut Criterion) {
    fs::create_dir_all(pth("")).expect("");
    fs::File::create(pth("10k"))
        .expect("")
        .write_all(&[0x61_u8; 10000])
        .expect("");
    fs::File::create(pth("10M"))
        .expect("")
        .write_all(&[0x62_u8; 10000000])
        .expect("");

    c.bench_function("fscopy 10k", |b| b.iter(|| fscopy(black_box("10k"))));
    c.bench_function("iocopy 10k", |b| b.iter(|| iocopy(black_box("10k"))));
    c.bench_function("fscopy 10M", |b| b.iter(|| fscopy(black_box("10M"))));
    c.bench_function("iocopy 10M", |b| b.iter(|| iocopy(black_box("10M"))));

    c.bench_function("copy_then_hash 10k", |b| {
        b.iter(|| copy_then_hash(black_box("10k")))
    });
    c.bench_function("tee_and_hash 10k", |b| {
        b.iter(|| tee_and_hash(black_box("10k")))
    });
    c.bench_function("copy_then_hash 10M", |b| {
        b.iter(|| copy_then_hash(black_box("10M")))
    });
    c.bench_function("tee_and_hash 10M", |b| {
        b.iter(|| tee_and_hash(black_box("10M")))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);

fn fscopy(filename: &str) {
    // Somewhere deep down, `fs::copy`` will attempt to use:
    //   `std::sys::pal::unix::kernel_copy::copy_regular_files()`
    // ... which isn't publicly exported.  (Sadface.)
    //
    // In theory, one might imagine this could result in using some
    // interface to the kernel which could get asymtotically different
    // results by expressing the higher level intention, and skipping
    // over all the dumb IO of copying bytes in and out of userspace.
    //
    // In practice, I do not see such gains in this benchmark.
    // And it's a tad unclear what I'd do if I did see them;
    // the `fs::copy` function *also* does other work that we... don't want to
    // (naming copying permissions; minor, but undesired).
    fs::copy(pth(filename), pth("out")).expect("");
}
fn iocopy(filename: &str) {
    let mut src = fs::File::open(pth(filename)).expect("");
    let mut dst = fs::File::create(pth("out")).expect("");
    io::copy(&mut src, &mut dst).expect("");
}

use sha2::Digest;

fn copy_then_hash(filename: &str) -> [u8; 32] {
    fs::copy(pth(filename), pth("out")).expect("");
    let mut src = fs::File::open(pth("out")).expect("");
    let mut hasher = sha2::Sha256::new();
    io::copy(&mut src, &mut hasher).expect("");
    hasher.finalize().into()
}
fn tee_and_hash(filename: &str) -> [u8; 32] {
    let src = fs::File::open(pth(filename)).expect("");
    let mut dst = fs::File::create(pth("out")).expect("");
    let mut hasher = sha2::Sha256::new();
    let mut tee = io_tee::TeeReader::new(src, &mut hasher);
    io::copy(&mut tee, &mut dst).expect("");
    hasher.finalize().into()
}
