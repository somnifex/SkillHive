use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const HASH_PREFIX: &str = "sha256:";
const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub hash: String,
    pub size_bytes: u64,
}

impl BlobStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, BlobStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("sha256"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Persists an immutable blob using write -> fsync -> atomic rename.
    ///
    /// The temporary file is created in the destination directory so the final
    /// rename cannot cross filesystem boundaries. Existing content with the
    /// same digest is reused only after its digest is verified.
    pub fn put_bytes(&self, bytes: &[u8]) -> Result<BlobRef, BlobStoreError> {
        let hash = hash_bytes(bytes);
        let destination = self.path_for_hash(&hash)?;

        if destination.exists() {
            if self.verify(&hash)? {
                return Ok(BlobRef {
                    hash,
                    size_bytes: bytes.len() as u64,
                });
            }
            return Err(BlobStoreError::CorruptedExistingBlob(hash));
        }

        let parent = destination
            .parent()
            .ok_or_else(|| BlobStoreError::InvalidHash(hash.clone()))?;
        fs::create_dir_all(parent)?;

        let temporary = parent.join(format!(".skillhive-blob-{}.tmp", Uuid::new_v4()));
        let write_result = (|| -> Result<(), BlobStoreError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);

            match fs::rename(&temporary, &destination) {
                Ok(()) => sync_parent_directory(parent)?,
                Err(error) if destination.exists() => {
                    // Another writer may have committed the same content first.
                    // Never replace a content-addressed object; verify and reuse.
                    if self.verify(&hash)? {
                        fs::remove_file(&temporary).ok();
                    } else {
                        return Err(BlobStoreError::Io(error));
                    }
                }
                Err(error) => return Err(BlobStoreError::Io(error)),
            }
            Ok(())
        })();

        if write_result.is_err() {
            fs::remove_file(&temporary).ok();
        }
        write_result?;

        Ok(BlobRef {
            hash,
            size_bytes: bytes.len() as u64,
        })
    }

    pub fn read_bytes(&self, hash: &str) -> Result<Vec<u8>, BlobStoreError> {
        let path = self.path_for_hash(hash)?;
        let bytes = fs::read(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                BlobStoreError::NotFound(hash.to_owned())
            } else {
                BlobStoreError::Io(error)
            }
        })?;

        let actual = hash_bytes(&bytes);
        if actual != hash {
            return Err(BlobStoreError::HashMismatch {
                expected: hash.to_owned(),
                actual,
            });
        }
        Ok(bytes)
    }

    pub fn verify(&self, hash: &str) -> Result<bool, BlobStoreError> {
        let path = self.path_for_hash(hash)?;
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(BlobStoreError::Io(error)),
        };
        Ok(hash_reader(&mut file)? == hash)
    }

    pub fn path_for_hash(&self, hash: &str) -> Result<PathBuf, BlobStoreError> {
        let digest = hash
            .strip_prefix(HASH_PREFIX)
            .ok_or_else(|| BlobStoreError::InvalidHash(hash.to_owned()))?;
        if digest.len() != SHA256_HEX_LEN
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(BlobStoreError::InvalidHash(hash.to_owned()));
        }

        Ok(self.root.join("sha256").join(&digest[..2]).join(digest))
    }
}

fn hash_reader(reader: &mut impl Read) -> Result<String, BlobStoreError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format_digest(hasher.finalize().as_slice()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format_digest(hasher.finalize().as_slice())
}

fn format_digest(digest: &[u8]) -> String {
    let mut output = String::with_capacity(HASH_PREFIX.len() + SHA256_HEX_LEN);
    output.push_str(HASH_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), BlobStoreError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), BlobStoreError> {
    // Windows does not expose portable directory fsync through std. The file
    // itself is fully flushed before rename; platform-specific hardening can be
    // introduced behind this boundary without changing BlobStore semantics.
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum BlobStoreError {
    #[error("blob filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid SHA-256 blob identifier: {0}")]
    InvalidHash(String),
    #[error("blob not found: {0}")]
    NotFound(String),
    #[error("existing content-addressed blob is corrupted: {0}")]
    CorruptedExistingBlob(String),
    #[error("blob integrity check failed: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_is_content_addressed_and_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(temp.path()).expect("store");

        let first = store.put_bytes(b"hello SkillHive").expect("first write");
        let second = store.put_bytes(b"hello SkillHive").expect("second write");

        assert_eq!(first, second);
        assert!(store.verify(&first.hash).expect("verify"));
        assert_eq!(store.read_bytes(&first.hash).expect("read"), b"hello SkillHive");
    }

    #[test]
    fn rejects_noncanonical_hashes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(temp.path()).expect("store");

        assert!(matches!(
            store.path_for_hash("abc123"),
            Err(BlobStoreError::InvalidHash(_))
        ));
        assert!(matches!(
            store.path_for_hash(
                "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            ),
            Err(BlobStoreError::InvalidHash(_))
        ));
    }

    #[test]
    fn detects_corruption_on_read() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = BlobStore::open(temp.path()).expect("store");
        let blob = store.put_bytes(b"original").expect("write");
        let path = store.path_for_hash(&blob.hash).expect("path");
        fs::write(path, b"corrupted").expect("corrupt");

        assert!(matches!(
            store.read_bytes(&blob.hash),
            Err(BlobStoreError::HashMismatch { .. })
        ));
    }
}
