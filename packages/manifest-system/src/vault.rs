//! Encrypted storage for the files of private instances (the "vault").
//!
//! Event mods and packs are kept on disk only in this form. The key comes from the website for each member
//! (`GET /api/launcher/instances/:id/vault-key`) and lives in memory, so the blobs are useless outside the
//! launcher, and a member who is removed can no longer open them.
//!
//! Format (`.tkv`): `TKV1` · key id length (u8) · key id · 8-byte random nonce prefix · chunks.
//! Each chunk is up to 1 MiB of plaintext sealed with AES-256-GCM (ciphertext + 16-byte tag). The nonce is
//! the prefix followed by a big-endian u32 counter whose top bit marks the last chunk, so chunks cannot be
//! reordered, dropped or truncated. The expected SHA-256 of the plaintext is the associated data, so a blob
//! cannot be swapped for another file of the same instance.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use thiserror::Error;

const MAGIC: &[u8; 4] = b"TKV1";
const CHUNK: usize = 1024 * 1024;
const TAG: usize = 16;
const LAST: u32 = 0x8000_0000;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("Clave del almacén no válida.")]
    BadKey,

    #[error("Archivo cifrado dañado o de otra clave.")]
    Corrupt,

    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type VaultResult<T> = Result<T, VaultError>;

/// A 32-byte AES key plus the server's identifier for it.
pub struct VaultKey {
    key: LessSafeKey,
    id: String,
}

impl VaultKey {
    pub fn new(raw: &[u8], id: &str) -> VaultResult<Self> {
        if raw.len() != 32 || id.is_empty() || id.len() > 32 {
            return Err(VaultError::BadKey);
        }
        let unbound = UnboundKey::new(&AES_256_GCM, raw).map_err(|_| VaultError::BadKey)?;
        Ok(Self {
            key: LessSafeKey::new(unbound),
            id: id.to_string(),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

fn nonce(prefix: &[u8; 8], counter: u32, last: bool) -> VaultResult<Nonce> {
    if counter & LAST != 0 {
        // 2^31 MiB: far beyond any file the website accepts.
        return Err(VaultError::Corrupt);
    }
    let mut bytes = [0u8; 12];
    bytes[..8].copy_from_slice(prefix);
    bytes[8..].copy_from_slice(&(counter | if last { LAST } else { 0 }).to_be_bytes());
    Ok(Nonce::assume_unique_for_key(bytes))
}

/// Fills `buf` as far as possible; returns how many bytes were read (less only at the end of the input).
fn fill(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut read = 0;
    while read < buf.len() {
        match reader.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(read)
}

/// Encrypts `plain` into `sealed` (written through a temporary file, then renamed). `sha256` is the
/// expected hash of the plaintext, already verified by the caller.
pub fn seal_file(key: &VaultKey, sha256: &str, plain: &Path, sealed: &Path) -> VaultResult<()> {
    let mut prefix = [0u8; 8];
    SystemRandom::new().fill(&mut prefix).map_err(|_| VaultError::BadKey)?;
    let tmp = sealed.with_extension("tmp");
    {
        let mut input = BufReader::new(File::open(plain)?);
        let mut output = BufWriter::new(File::create(&tmp)?);
        output.write_all(MAGIC)?;
        output.write_all(&[key.id.len() as u8])?;
        output.write_all(key.id.as_bytes())?;
        output.write_all(&prefix)?;
        let mut buf = vec![0u8; CHUNK];
        let mut counter = 0u32;
        loop {
            let n = fill(&mut input, &mut buf)?;
            let last = n < CHUNK || input.fill_buf()?.is_empty();
            let mut chunk = buf[..n].to_vec();
            key.key
                .seal_in_place_append_tag(nonce(&prefix, counter, last)?, Aad::from(sha256.as_bytes()), &mut chunk)
                .map_err(|_| VaultError::BadKey)?;
            output.write_all(&chunk)?;
            if last {
                break;
            }
            counter += 1;
        }
        output.flush()?;
        output.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, sealed)?;
    Ok(())
}

/// Key id a sealed file was written with, or `None` if it is not a vault file.
pub fn sealed_key_id(sealed: &Path) -> io::Result<Option<String>> {
    let mut file = File::open(sealed)?;
    let mut head = [0u8; 5];
    if fill(&mut file, &mut head)? < 5 || &head[..4] != MAGIC {
        return Ok(None);
    }
    let mut id = vec![0u8; head[4] as usize];
    if fill(&mut file, &mut id)? < id.len() {
        return Ok(None);
    }
    Ok(String::from_utf8(id).ok())
}

/// Decrypts `sealed` into `plain` (through a temporary file). Fails with [`VaultError::Corrupt`] if anything
/// was modified, the blob is for another file or it was sealed with another key.
pub fn open_file(key: &VaultKey, sha256: &str, sealed: &Path, plain: &Path) -> VaultResult<()> {
    let mut input = BufReader::new(File::open(sealed)?);
    let mut head = [0u8; 5];
    if fill(&mut input, &mut head)? < 5 || &head[..4] != MAGIC {
        return Err(VaultError::Corrupt);
    }
    let mut id = vec![0u8; head[4] as usize];
    if fill(&mut input, &mut id)? < id.len() || id != key.id.as_bytes() {
        return Err(VaultError::Corrupt);
    }
    let mut prefix = [0u8; 8];
    if fill(&mut input, &mut prefix)? < 8 {
        return Err(VaultError::Corrupt);
    }

    let tmp = plain.with_file_name(format!(
        ".{}.tmp",
        plain.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    ));
    let result = (|| {
        let mut output = BufWriter::new(File::create(&tmp)?);
        let mut buf = vec![0u8; CHUNK + TAG];
        let mut counter = 0u32;
        loop {
            let n = fill(&mut input, &mut buf)?;
            let last = n < CHUNK + TAG || input.fill_buf()?.is_empty();
            if n < TAG {
                return Err(VaultError::Corrupt);
            }
            let opened = key
                .key
                .open_in_place(
                    nonce(&prefix, counter, last)?,
                    Aad::from(sha256.as_bytes()),
                    &mut buf[..n],
                )
                .map_err(|_| VaultError::Corrupt)?;
            output.write_all(opened)?;
            if last {
                break;
            }
            counter += 1;
        }
        output.flush()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if plain.exists() {
        std::fs::remove_file(plain)?;
    }
    std::fs::rename(&tmp, plain)?;
    Ok(())
}

/// Writes `comment` as the archive comment of the zip/jar at `path` (replacing any existing one). Java,
/// Fabric and Minecraft ignore it, so the file keeps working; it says who the copy was licensed to.
/// Returns `false` (and leaves the file alone) when it is not a zip.
pub fn set_zip_comment(path: &Path, comment: &str) -> io::Result<bool> {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    let comment = &comment.as_bytes()[..comment.len().min(u16::MAX as usize)];
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();
    let tail_len = len.min(22 + u16::MAX as u64);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    // The end-of-central-directory record is the last one whose comment length reaches the end of the file.
    let found = (0..tail.len().saturating_sub(21)).rev().find(|&at| {
        tail[at..at + 4] == EOCD && {
            let existing = u16::from_le_bytes([tail[at + 20], tail[at + 21]]) as usize;
            at + 22 + existing == tail.len()
        }
    });
    let Some(at) = found else { return Ok(false) };
    let eocd = len - tail_len + at as u64;
    file.set_len(eocd + 22)?;
    file.seek(SeekFrom::Start(eocd + 20))?;
    file.write_all(&(comment.len() as u16).to_le_bytes())?;
    file.write_all(comment)?;
    file.sync_all()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        let mut bytes = [0u8; 8];
        SystemRandom::new().fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("takumi-vault-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key(byte: u8) -> VaultKey {
        VaultKey::new(&[byte; 32], "k1").unwrap()
    }

    #[test]
    fn round_trips_files_of_every_size() {
        let dir = dir();
        for size in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK + 7] {
            let data: Vec<u8> = (0..size).map(|i| (i * 31 % 251) as u8).collect();
            let plain = dir.join("plain");
            std::fs::write(&plain, &data).unwrap();
            seal_file(&key(1), "abc", &plain, &dir.join("blob.tkv")).unwrap();
            let sealed = std::fs::read(dir.join("blob.tkv")).unwrap();
            assert!(
                size < 64 || !sealed.windows(64).any(|w| w == &data[..64]),
                "plaintext leaked"
            );
            assert_eq!(sealed_key_id(&dir.join("blob.tkv")).unwrap().as_deref(), Some("k1"));
            open_file(&key(1), "abc", &dir.join("blob.tkv"), &dir.join("out")).unwrap();
            assert_eq!(std::fs::read(dir.join("out")).unwrap(), data, "size {size}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_wrong_key_wrong_file_and_tampering() {
        let dir = dir();
        let data = vec![7u8; 2 * CHUNK + 10];
        std::fs::write(dir.join("plain"), &data).unwrap();
        let blob = dir.join("blob.tkv");
        seal_file(&key(1), "abc", &dir.join("plain"), &blob).unwrap();
        let out = dir.join("out");
        assert!(matches!(
            open_file(&key(2), "abc", &blob, &out),
            Err(VaultError::Corrupt)
        ));
        assert!(matches!(
            open_file(&key(1), "abd", &blob, &out),
            Err(VaultError::Corrupt)
        ));
        assert!(!out.exists(), "nothing is written on failure");

        let original = std::fs::read(&blob).unwrap();
        let mut flipped = original.clone();
        let at = flipped.len() / 2;
        flipped[at] ^= 1;
        std::fs::write(&blob, &flipped).unwrap();
        assert!(matches!(
            open_file(&key(1), "abc", &blob, &out),
            Err(VaultError::Corrupt)
        ));

        // Dropping the last chunk is detected: the new last chunk was not sealed as last.
        let header = 4 + 1 + 2 + 8;
        std::fs::write(&blob, &original[..header + 2 * (CHUNK + TAG)]).unwrap();
        assert!(matches!(
            open_file(&key(1), "abc", &blob, &out),
            Err(VaultError::Corrupt)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn zip_comment_marks_the_copy_and_keeps_the_archive_valid() {
        let dir = dir();
        let path = dir.join("mod.jar");
        {
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"{}").unwrap();
            zip.finish().unwrap();
        }
        assert!(set_zip_comment(&path, "Licenciado a steve").unwrap());
        assert!(set_zip_comment(&path, "Licenciado a alex · evento").unwrap());
        let archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.comment(), "Licenciado a alex · evento".as_bytes());
        assert_eq!(archive.len(), 1);

        std::fs::write(dir.join("text.txt"), b"not a zip").unwrap();
        assert!(!set_zip_comment(&dir.join("text.txt"), "x").unwrap());
        assert_eq!(std::fs::read(dir.join("text.txt")).unwrap(), b"not a zip");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
