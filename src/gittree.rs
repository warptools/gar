use bytes::BufMut;
use bytes::BytesMut;
use sha2::Digest;
use std::fmt::Debug;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(PartialEq)]
struct Hash([u8; 32]);

impl Debug for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Hash").field(&hex::encode(&self.0)).finish()
    }
}

impl Hash {
    fn from_hex<S: AsRef<[u8]>>(hex: S) -> Result<Self, hex::FromHexError> {
        let mut out = [0u8; 32];
        hex::decode_to_slice(hex, &mut out)?;
        Ok(Self(out))
    }
}

fn hash_of_stream<R>(reader: &mut R, claimed_size: u64) -> Result<Hash, io::Error>
where
    R: io::Read + ?Sized,
{
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"blob ");
    hasher.update(format!("{}", claimed_size));
    hasher.update([0]);

    let bytes_written = io::copy(reader, &mut hasher)?;
    if bytes_written != claimed_size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "{} bytes in stream, but expected {}",
                bytes_written, claimed_size
            ),
        ));
    }
    let hash_bytes = hasher.finalize();
    Ok(Hash(hash_bytes.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn test_hash_blob_fixture() {
        let hash = hash_of_stream(&mut io::Cursor::new(b"a file\n"), 7).expect("hash to succeed");
        assert_eq!(
            hex::encode(&hash.0),
            "2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce"
        );
    }

    #[rstest]
    // Just a plain file.
    #[case("fixtures/alpha/a_file", Hash::from_hex("2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce").expect(""))]
    #[case("fixtures/alpha/a_dir/other_file", Hash::from_hex("8431d03990244d0bffa3dfecdd7a67d0bca2f5e999bff04469cde93cc2365d96").expect(""))]
    #[case("fixtures/alpha/a_dir/more_files", Hash::from_hex("4698ba4d7c51602d6a50e4fb6e150e2e06d625ba5874cde627bc6dfc357a23db").expect(""))]
    #[case("fixtures/alpha/a_dir/deeper/samefile", Hash::from_hex("4698ba4d7c51602d6a50e4fb6e150e2e06d625ba5874cde627bc6dfc357a23db").expect(""))]
    // Dir with one file.  (Sorting thus can't be the problem, if this fixture fails.)
    #[case("fixtures/alpha/a_dir/deeper", Hash::from_hex("9897054d9f01c666ac1371d3e0a022a67b5df59ddb1608e8165a3b1fa22da706").expect(""))]
    // Dir with files and subdirs.
    #[case("fixtures/alpha/a_dir", Hash::from_hex("e1896fb25dd721b447c52e40267a90405ebc41aaa2c7143e9cf58cf5c8421cde").expect(""))]
    // A wild symlink appears!
    #[case("fixtures/alpha/a_symlink", Hash::from_hex("45a01848912e900ef582a23ef763c77ddb2d955bea7756072fb056f43534fca8").expect(""))]
    // Dir with multiple files (sorting matters), symlinks, and subdirs (including recursively).
    #[case("fixtures/alpha", Hash::from_hex("9024a7f8afa43db06ff2b50d9ac9c21b791bee49d8092d3f14f1e433bfd927fa").expect(""))]
    fn test_hash_of_path(#[case] path: String, #[case] expected: Hash) {
        assert_eq!(expected, hash_of_path(path).expect("no io errors"))
    }
}

fn hash_of_path<P: AsRef<Path>>(path: P) -> Result<Hash, io::Error> {
    let metadata = path.as_ref().symlink_metadata()?;
    // FileType isn't an enum (imagine: its membership size would vary per platform if it was!)
    // so working with it ends up being a series of unappealing "if" blocks rather than a nice clean exhaustive match.
    if metadata.is_file() {
        return Ok(hash_of_stream(&mut fs::File::open(path)?, metadata.size())?);
    }
    if metadata.is_symlink() {
        todo!()
    }
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, io::Error>>()?;
        entries.sort_by(|a, b| a.path().partial_cmp(&b.path()).unwrap());

        // We have to buffer descriptions of all children, because the git format writes the serial size of that in a header.
        // Start accumulating the buffer.
        // We can make a pretty good guess how big it'll need to be, at most:
        let size_per_ent = 7 + 255 + 1 + 32;
        let mut buf = BytesMut::with_capacity(entries.len() * size_per_ent);

        // For each entry: recurse on hashing; append buffer.
        for ent in entries.iter() {
            let hash = hash_of_path(ent.path())?;
            let ft = ent.file_type()?;
            // First write the type info.
            if ft.is_file() {
                // Asking if it's executable is rather graceful in Rust...
                if ent.metadata()?.permissions().mode() & 0x111 > 0 {
                    buf.put(&b"100755 "[..])
                } else {
                    buf.put(&b"100644 "[..])
                }
            } else if ft.is_symlink() {
                buf.put(&b"120000 "[..])
            } else if ft.is_dir() {
                buf.put(&b"40000 "[..]) // This certainly looks like a typo, doesn't it!  But, indeed... this is exactly how git encodes this.
            } else {
                panic!("unknown file type")
            }
            // Now the name (and a terminating delimiter).
            buf.put::<&[u8]>(ent.file_name().as_os_str().as_encoded_bytes());
            buf.put_u8(0);
            // Now its hash.
            buf.put(&hash.0[..])
            // Somewhat shockingly, there's no further delimiter here.  The hash length is necessarily hardcoded by this absense.
        }

        eprintln!("debug: tree buff is -->{buf:?}<--");

        // Ultimately: hash the preamble and feed the buffer and finalize.
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"tree ");
        hasher.update(format!("{}", buf.len()));
        hasher.update([0]);
        hasher.update(buf);
        let hash_bytes = hasher.finalize();
        return Ok(Hash(hash_bytes.into()));
    }
    panic!("unknown file type")
}