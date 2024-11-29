File Copy Strategy Benchmark Notes
==================================

The Question
------------

Is it useful to use the `fs::copy` method, which _may_ internally use some direct copy kernel APIs, worth anything?

And considering that we need to read a file into memory at least once to hash it,
do those (maybe) savings of (maybe) using a more direct kernel copy API still win any tugs of war overall,
considering that they necessitate a second set of `read` syscalls?


Summary of Data
---------------

We do two sets of studies at various file sizes:
one is of the `fs::copy` vs `io::copy` APIs alone;
and the other is with a full hashing implemented similarly to how our real program would need to do it
(but ignoring all the details of git trees, etc; just using sha256 and that's it).

Observing fscopy vs iocopy alone:

- on tmpfs, it doesn't matter, at any scale.
- on ZFS, it doesn't matter, at any scale.
- on Ext4... you might be tempted to think it matters, but ext4 is so unpredictable and noisy that statistics involving it are just a joke.

Observing the difference between fscopy-then-reread-and-hash and using an in-memory tee to hash and copy:

- on tmpfs, it doesn't matter.  Tee is maybe _slightly_ winning but practically within noise margins.
- on ZFS, it looks like copy-and-reread is slightly faster -- not an asymptote, but by percentages (approximately 19%)... on 10M and above.  Tee is faster on 10k.
- on Ext4... yeah, these measurements are just insane.  Ext4 is unpredictable beyond reason.

(The lack of comprehensible measurements of ext4 is concerning, but see data below.
If anyone can make sense of this, by all means, please send notes.)


Conclusions
-----------

- I see no meaningful evidence that the fs::copy of the standard library
  gets any advantages from its internal `unix::kernel_copy` features on any filesystem I tested.
- Writing more complex code to do an in-memory tee does not seem to yield any benefits vs doing a second read.

I struggle to explain why copy-and-reread would be faster when the fscopy sample alone *isn't*.
I guess the only plausible theory is... the tee library is not very efficient, or that it results in worse IO pipelining
(and, that this doesn't matter on a tmpfs, because... the writes are cheaper and don't pipeline stall as much...?).

If the above theory is accurate, then the most useful thing to do to make this suite of operations go faster
might actually be multi-threading.
(However, that is a whole other series of even more tricky benchmarks.)


Data
----

Here's a set of tables of what I see on my sample device,
on various filesystems.
Numbers are the mean (the middle number in the Criterion report).
(The outliers and variation are *considerable* but I see little point to fretting them.)
(Numbers included for both during battery operation and with power, but not interesting;
as a general rule, things are twice as slow on battery.)

#### On tmpfs (with power, and on battery):

```
copying/fscopy/10k      5.3987 µs     10.544 µs
copying/iocopy/10k      5.4841 µs     10.504 µs
copying/fscopy/10M      3.3739 ms     5.7873 ms
copying/iocopy/10M      3.3786 ms     5.7336 ms
copying/fscopy/100M     40.111 ms     65.658 ms
copying/iocopy/100M     40.145 ms     65.554 ms
cp+hash/cp+re/10k       12.763 µs     25.913 µs
cp+hash/tee/10k         10.991 µs     22.218 µs
cp+hash/cp+re/10M       11.204 ms     21.778 ms
cp+hash/tee/10M         11.029 ms     21.597 ms
cp+hash/cp+re/100M      121.94 ms     229.00 ms
cp+hash/tee/100M        115.92 ms     220.97 ms
```

#### On ZFS (with power, and on battery):

```
copying/fscopy/10k      20.728 µs     43.884 µs
copying/iocopy/10k      18.616 µs     39.305 µs
copying/fscopy/10M      2.6332 ms     4.8568 ms
copying/iocopy/10M      2.4318 ms     4.9970 ms
copying/fscopy/100M     28.903 ms     54.729 ms
copying/iocopy/100M     29.834 ms     53.218 ms
cp+hash/cp+re/10k       31.433 µs     61.972 µs
cp+hash/tee/10k         28.913 µs     60.354 µs
cp+hash/cp+re/10M       11.122 ms     22.667 ms
cp+hash/tee/10M         13.354 ms     27.027 ms
cp+hash/cp+re/100M      117.24 ms     230.44 ms
cp+hash/tee/100M        136.34 ms     271.09 ms
```

#### On Ext4 (with power, and on battery):

```
copying/fscopy/10k      22.201 µs     41.010 µs
copying/iocopy/10k      21.921 µs     40.659 µs
copying/fscopy/10M      7.3740 ms     66.556 ms
copying/iocopy/10M      21.904 ms     67.636 ms
copying/fscopy/100M     80.853 ms     726.81 ms
copying/iocopy/100M     556.10 ms     587.81 ms
cp+hash/cp+re/10k       57.929 µs     54.572 µs
cp+hash/tee/10k         47.561 µs     105.08 µs
cp+hash/cp+re/10M       65.870 ms     59.610 ms
cp+hash/tee/10M         64.434 ms     27.111 ms
cp+hash/cp+re/100M      595.20 ms     274.86 ms
cp+hash/tee/100M        239.08 ms     321.11 ms
```

And another round on power, because this erraticness is hard to believe:

```
copying/fscopy/10k      time:   [23.279 µs 23.676 µs 24.080 µs]
copying/iocopy/10k      time:   [24.175 µs 24.492 µs 24.802 µs]
copying/fscopy/10M      time:   [7.5497 ms 7.6162 ms 7.6876 ms]
copying/iocopy/10M      time:   [8.2591 ms 9.8872 ms 11.877 ms]
copying/fscopy/100M     time:   [409.51 ms 473.17 ms 536.48 ms]
copying/iocopy/100M     time:   [122.48 ms 161.24 ms 205.90 ms]
cp+hash/cp+re/10k       time:   [25.219 µs 25.550 µs 25.923 µs]
cp+hash/tee/10k         time:   [29.991 µs 30.362 µs 30.753 µs]
cp+hash/cp+re/10M       time:   [27.476 ms 33.347 ms 39.597 ms]
cp+hash/tee/10M         time:   [15.246 ms 15.342 ms 15.452 ms]
cp+hash/cp+re/100M      time:   [167.83 ms 169.03 ms 170.20 ms]
cp+hash/tee/100M        time:   [173.97 ms 192.83 ms 215.71 ms]
```

The erraticness of Ext4 remains inscruitable after multiple rounds.
Observe how `copying/fscopy/100M` is sometimes found being a fraction of the iocopy time,
but is in two other cases (one powered and one battery) iocopy is the winner,
once by 50% and once by 3x.
Various other numbers are similarly wildly inconsistent...
and this also despite not having nearly that high of variation within a test run.
I have simply no idea what to make of this,
other than to conclude Ext4 has such unpredictable performance that measuring it is
more or less a waste of time and sanity.
