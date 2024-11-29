/*
The purpose of this file is to probe into if it's a better idea to
do any file copying first, and hashing with a subsequent read;
or if it's preferable to read, tee in memory, and hash and write at once.

Perhaps surprisingly, the result in appears to be:
copy, and then read again.
There are kernel APIs for copying files efficiently;
and that means then a subsequent read for the purpose of hashing
isn't not so much a "second" read as it is simply a first one.
Empirically, the "second" read approach is either on par or faster than
an in-memory tee approach.

See adjacent markdown file of the same basename for more notes and some captured data.

All of this could also be reviewed in contrast with rustix functions, if we add that.
I haven't studied it yet, but it's possible that crate would offer different IO abstraction levels.
However, it does seem like both fs::copy and io::copy are already deploying all the wizardry they can.

*/

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::io;
use std::{fs, io::Write, path::PathBuf};

fn pth(filename: &str) -> PathBuf {
    PathBuf::from("/tmp/gar-test/").join(filename)
}

pub fn criterion_benchmark(c: &mut Criterion) {
    const K: usize = 1204;
    const M: usize = K * K;
    fs::create_dir_all(pth("")).expect("");
    fs::File::create(pth("10k"))
        .expect("")
        .write_all(&[0x61_u8; 10 * K])
        .expect("");
    fs::File::create(pth("10M"))
        .expect("")
        .write_all(&[0x62_u8; 10 * M])
        .expect("");
    fs::File::create(pth("100M"))
        .expect("")
        .write_all(&[0x62_u8; 100 * M])
        .expect("");
    let sizes = ["10k", "10M", "100M"];

    let mut group = c.benchmark_group("copying");
    for i in sizes.iter() {
        group.bench_with_input(BenchmarkId::new("fscopy", i), i, |b, i| {
            b.iter(|| fscopy(*i))
        });
        group.bench_with_input(BenchmarkId::new("iocopy", i), i, |b, i| {
            b.iter(|| iocopy(*i))
        });
    }
    group.finish();

    // Strings here shortened because criterion starts putting more linebreaks in output
    // if names go over a certain magical length, which makes it even harder to parse criterion's already noisy output.
    // Wishlist: research other benchmarking frameworks, because these are dumb, invented problems.
    let mut group = c.benchmark_group("cp+hash");
    for i in sizes.iter() {
        group.bench_with_input(BenchmarkId::new("cp+re", i), i, |b, i| {
            b.iter(|| copy_then_hash(*i))
        });
        group.bench_with_input(BenchmarkId::new("tee", i), i, |b, i| {
            b.iter(|| tee_and_hash(*i))
        });
    }
    group.finish();
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
    // However, (see below) it turns out `io::copy` *also* does this.
    // And the `fs::copy` function does some additional work
    // (namingly copying permissions) that we don't necessarily desire.
    //
    // Comparing `fs::copy` and `io::copy` turns out to be not super interesting,
    // because they turn into the same thing.  But I leave the benchmarks here
    // for posterity, comedy, and interest.
    fs::copy(pth(filename), pth("out")).expect("");
}
fn iocopy(filename: &str) {
    // This... turns out not to test what you might thing.
    // `io::copy` has special powers, too, and detects arguments that are files.
    // (Furthermore, no, wrapping it in e.g. `io::BufWriter::new()` appears to make
    // no difference either; the detection and the magic still kicks in, as far as I can tell.)
    //
    // `io::copy` calls into `std::sys::pal::unix::kernel_copy::copy_spec`.
    // This is actually a superset of where `fs::copy` goes with `copy_regular_files`:
    // `copy_regular_files` gets used inside `copy_spec`,
    // and subsequently several other fancy tricks (such as sendfile for mmapables) are also tried.
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
