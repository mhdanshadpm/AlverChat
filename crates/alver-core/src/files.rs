//! Bounded, hash-verified file reception for LAN chunk transfers.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::protocol::MAX_FILE_CHUNK_BYTES;

pub const DEFAULT_MAX_FILE_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileOffer {
    pub id: Uuid,
    pub file_name: String,
    pub size: u64,
    pub blake3_hash: [u8; 32],
}

impl FileOffer {
    pub fn validate(&self, max_file_bytes: u64) -> Result<(), String> {
        if self.size > max_file_bytes {
            return Err("file offer exceeds the configured size limit".into());
        }
        if self.file_name.is_empty()
            || self.file_name == "."
            || self.file_name == ".."
            || self.file_name.contains('/')
            || self.file_name.contains('\\')
            || self.file_name.contains('\0')
        {
            return Err("file name must be a single safe path component".into());
        }
        Ok(())
    }

    pub fn chunk_count(&self) -> u64 {
        self.size.div_ceil(MAX_FILE_CHUNK_BYTES as u64)
    }
}

pub struct ChunkReceiver {
    offer: FileOffer,
    part_path: PathBuf,
    destination_root: PathBuf,
    file: File,
    received: BTreeSet<u64>,
}

impl ChunkReceiver {
    pub fn new(
        destination_root: &Path,
        offer: FileOffer,
        max_file_bytes: u64,
    ) -> Result<Self, String> {
        offer.validate(max_file_bytes)?;
        let destination_root =
            fs::canonicalize(destination_root).map_err(|error| error.to_string())?;
        if !destination_root.is_dir() {
            return Err("download destination is not a directory".into());
        }
        let part_path = destination_root.join(format!(".alverchat-{}.part", offer.id));
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(&part_path)
            .map_err(|error| error.to_string())?;
        file.set_len(offer.size)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            offer,
            part_path,
            destination_root,
            file,
            received: BTreeSet::new(),
        })
    }

    pub fn write_chunk(&mut self, index: u64, bytes: &[u8]) -> Result<bool, String> {
        let total = self.offer.chunk_count();
        if index >= total {
            return Err("chunk index is outside the offered file".into());
        }
        let offset = index
            .checked_mul(MAX_FILE_CHUNK_BYTES as u64)
            .ok_or_else(|| "chunk offset overflow".to_owned())?;
        let expected_len = (self.offer.size - offset).min(MAX_FILE_CHUNK_BYTES as u64) as usize;
        if bytes.len() != expected_len || bytes.len() > MAX_FILE_CHUNK_BYTES {
            return Err("chunk length does not match the offered file".into());
        }
        if self.received.contains(&index) {
            return Ok(false);
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.write_all(bytes))
            .map_err(|error| error.to_string())?;
        self.received.insert(index);
        Ok(true)
    }

    pub fn finish(mut self) -> Result<PathBuf, String> {
        if self.received.len() as u64 != self.offer.chunk_count() {
            return Err("file transfer is missing chunks".into());
        }
        self.file.sync_all().map_err(|error| error.to_string())?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0_u8; 32 * 1024];
        loop {
            let count = self
                .file
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        if hasher.finalize().as_bytes() != &self.offer.blake3_hash {
            return Err("file hash does not match the offer".into());
        }
        let file_name: String = self
            .offer
            .file_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        let destination = self
            .destination_root
            .join(format!("{}-{file_name}", self.offer.id));
        drop(self.file);
        fs::rename(&self.part_path, &destination).map_err(|error| error.to_string())?;
        Ok(destination)
    }

    pub fn cancel(self) -> Result<(), String> {
        fs::remove_file(self.part_path).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("alverchat-files-{}", Uuid::now_v7()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn verifies_chunks_hash_and_sanitizes_destination_name() {
        let root = temp_dir();
        let contents = b"hello local network";
        let offer = FileOffer {
            id: Uuid::now_v7(),
            file_name: "team report?.txt".into(),
            size: contents.len() as u64,
            blake3_hash: *blake3::hash(contents).as_bytes(),
        };
        let mut receiver =
            ChunkReceiver::new(&root, offer.clone(), DEFAULT_MAX_FILE_BYTES).unwrap();
        assert!(receiver.write_chunk(0, contents).unwrap());
        assert!(!receiver.write_chunk(0, contents).unwrap());
        let path = receiver.finish().unwrap();
        assert!(path.starts_with(fs::canonicalize(&root).unwrap()));
        assert_eq!(fs::read(&path).unwrap(), contents);
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("team_report_.txt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsafe_names_oversize_chunks_and_bad_hashes() {
        let mut offer = FileOffer {
            id: Uuid::now_v7(),
            file_name: "../secret.txt".into(),
            size: 1,
            blake3_hash: *blake3::hash(b"x").as_bytes(),
        };
        assert!(offer.validate(DEFAULT_MAX_FILE_BYTES).is_err());
        offer.file_name = "valid.txt".into();
        assert!(offer.validate(0).unwrap_err().contains("size limit"));

        let root = temp_dir();
        offer.blake3_hash = *blake3::hash(b"expected").as_bytes();
        let mut receiver = ChunkReceiver::new(&root, offer, DEFAULT_MAX_FILE_BYTES).unwrap();
        assert!(receiver.write_chunk(0, b"bad").is_err());
        receiver.write_chunk(0, b"x").unwrap();
        assert!(receiver.finish().unwrap_err().contains("hash"));
        fs::remove_dir_all(root).unwrap();
    }
}
