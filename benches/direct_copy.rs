/*
The purpose of this file is to probe into if it makes any difference
to try to use `fs::copy` (so that it can use `unix::kernel_copy` features)
vs doing fairly dumb shuttling of IO through userspace.

The answer appears to be "no" -- in both directions.
`fs::copy` doesn't appear to have advantages over `io::copy`.
And there's mixed evidence on if teeing or vs a separate (arguably duplicate) read has any advantage...
(if anything, it seems... the opposite).

See adjacent markdown file of the same basename for more notes and some captured data.

All of this could also be reviewed in contrast with rustix functions, if we add that.
I haven't studied it yet, but it's possible that crate would offer different IO abstraction levels.

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
