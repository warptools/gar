use sha2::Digest;
use std::io;

struct Hash([u8; 32]);

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

    #[test]
    fn hash_blob_fixture() {
        let hash = hash_of_stream(&mut io::Cursor::new(b"a file\n"), 7).expect("hash to succeed");
        assert_eq!(
            hex::encode(&hash.0),
            "2909489adcb095aa795a9a7e6d92db735d0a0ced0782c43496675bdb7beec3ce"
        );
    }
}
