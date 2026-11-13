//! Bounded, hash-verified file reception for LAN chunk transfers.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{protocol::MAX_FILE_CHUNK_BYTES, security};

pub const DEFAULT_MAX_FILE_BYTES: u64 = 500 * 1024 * 1024;
/// Offers at or below this size download without waiting for an explicit accept.
pub const DEFAULT_AUTO_DOWNLOAD_LIMIT_BYTES: u64 = 25 * 1024 * 1024;

/// One persisted "this chunk is on disk" record inside the resume index.
const INDEX_RECORD_BYTES: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileOffer {
    pub id: Uuid,
    pub file_name: String,
    pub size: u64,
    pub blake3_hash: [u8; 32],
}

impl FileOffer {
    /// Check an offer against the local hard limit and the single-component name rule.
    ///
    /// The name check delegates to [`security::is_safe_single_component`] so a received
    /// offer can never smuggle an absolute path, traversal, control character or Windows
    /// device name into the download directory.
    pub fn validate(&self, max_file_bytes: u64) -> Result<(), String> {
        if self.size > max_file_bytes {
            return Err("file offer exceeds the configured size limit".into());
        }
        if !security::is_safe_single_component(&self.file_name) {
            return Err("file name must be a single safe path component".into());
        }
        Ok(())
    }

    pub fn chunk_count(&self) -> u64 {
        self.size.div_ceil(MAX_FILE_CHUNK_BYTES as u64)
    }
}

/// Byte length the chunk at `index` must have, or `None` when the index is out of range.
#[must_use]
pub fn expected_chunk_len(size: u64, chunk_count: u64, index: u64) -> Option<usize> {
    if index >= chunk_count {
        return None;
    }
    let offset = chunk_offset(index)?;
    if offset >= size {
        return None;
    }
    Some((size - offset).min(MAX_FILE_CHUNK_BYTES as u64) as usize)
}

/// File offset of chunk `index`, guarding against multiplication overflow.
#[must_use]
pub fn chunk_offset(index: u64) -> Option<u64> {
    index.checked_mul(MAX_FILE_CHUNK_BYTES as u64)
}

fn sanitize_file_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn final_name(offer: &FileOffer) -> String {
    format!("{}-{}", offer.id, sanitize_file_name(&offer.file_name))
}

fn hash_file(file: &mut File) -> Result<[u8; 32], String> {
    file.sync_all().map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// The lifecycle states a file transfer moves through.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    /// Nothing offered yet.
    Idle,
    /// An offer is waiting for the local user or the auto-download rule.
    Offered,
    /// Locally approved; chunks may start flowing.
    Accepted,
    /// Actively receiving chunks.
    Transferring,
    /// Deliberately stopped; verified chunks stay on disk.
    Paused,
    /// The whole-file blake3 hash matched the offer and the file is in place.
    Completed,
    /// Refused, aborted or hash-bad; untrusted bytes were discarded.
    Failed,
    /// Cancelled by a user; no files remain.
    Cancelled,
}

impl TransferState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Offered => "offered",
            Self::Accepted => "accepted",
            Self::Transferring => "transferring",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let found = match value {
            "idle" => Self::Idle,
            "offered" => Self::Offered,
            "accepted" => Self::Accepted,
            "transferring" => Self::Transferring,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        };
        Some(found)
    }

    /// States that no action may leave.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }

    /// Pure transition table: does `action` apply while in this state?
    #[must_use]
    pub fn allows(self, action: TransferAction) -> bool {
        match action {
            TransferAction::Receive => matches!(self, Self::Idle),
            TransferAction::Accept => matches!(self, Self::Idle | Self::Offered),
            TransferAction::Refuse => matches!(self, Self::Idle | Self::Offered | Self::Accepted),
            TransferAction::Begin => matches!(self, Self::Accepted),
            TransferAction::Pause => matches!(self, Self::Transferring),
            TransferAction::Resume => matches!(self, Self::Paused),
            TransferAction::Retry => matches!(self, Self::Failed),
            TransferAction::Chunk => matches!(self, Self::Transferring),
            TransferAction::Finish => matches!(self, Self::Transferring),
            TransferAction::Cancel => !self.is_terminal(),
        }
    }
}

/// Transitions a transfer can be asked to take.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    Receive,
    Accept,
    Refuse,
    Begin,
    Pause,
    Resume,
    Retry,
    Chunk,
    Finish,
    Cancel,
}

impl TransferAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Receive => "receive",
            Self::Accept => "accept",
            Self::Refuse => "refuse",
            Self::Begin => "begin",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Retry => "retry",
            Self::Chunk => "write_chunk",
            Self::Finish => "finish",
            Self::Cancel => "cancel",
        }
    }
}

/// Size limits a transfer honours; both come from store settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferPolicy {
    /// Offers above this are refused outright.
    pub max_file_bytes: u64,
    /// Offers above this wait for an explicit [`Transfer::accept`].
    pub auto_download_limit_bytes: u64,
}

impl Default for TransferPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            auto_download_limit_bytes: DEFAULT_AUTO_DOWNLOAD_LIMIT_BYTES,
        }
    }
}

impl TransferPolicy {
    /// The auto-download limit never sits above the hard cap.
    #[must_use]
    pub fn new(max_file_bytes: u64, auto_download_limit_bytes: u64) -> Self {
        Self {
            max_file_bytes,
            auto_download_limit_bytes: auto_download_limit_bytes.min(max_file_bytes),
        }
    }

    /// True when `size` may download without asking the user first.
    #[must_use]
    pub fn auto_downloads(self, size: u64) -> bool {
        size <= self.auto_download_limit_bytes
    }
}

/// Byte and chunk accounting for an in-flight transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub received_chunks: u64,
    pub total_chunks: u64,
    pub received_bytes: u64,
    pub total_bytes: u64,
    /// Whole percentage, 0..=100.
    pub percent: u32,
}

/// What the sender must resend after a pause, resume or retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResumePlan {
    /// First chunk index not yet verified on disk.
    pub next_index: u64,
    pub received_chunks: u64,
    pub total_chunks: u64,
    pub missing_chunks: u64,
    pub progress: Progress,
}

/// A resumable, bounded, hash-verified receive for one [`FileOffer`].
///
/// [`ChunkReceiver`] stays the simple single-session path. This type adds the
/// Idle/Offered/Accepted/Transferring/Paused/Completed|Failed|Cancelled machine and keeps
/// the verified chunk set in a sidecar index beside the part file, so a pause, a crash or
/// a reconnect reuses bytes already on disk. Success is only declared after the whole file
/// is re-hashed with blake3 against the offer.
pub struct Transfer {
    offer: FileOffer,
    policy: TransferPolicy,
    state: TransferState,
    reason: Option<String>,
    destination_root: PathBuf,
    part_path: PathBuf,
    index_path: PathBuf,
    file: Option<File>,
    received: BTreeSet<u64>,
}

impl Transfer {
    /// Prepare a transfer for `offer` under `policy`; nothing is created yet.
    pub fn incoming(
        destination_root: &Path,
        offer: FileOffer,
        policy: TransferPolicy,
    ) -> Result<Self, String> {
        let root = fs::canonicalize(destination_root).map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err("download destination is not a directory".into());
        }
        let prefix = format!(".alverchat-{}-resume", offer.id);
        Ok(Self {
            offer,
            policy,
            state: TransferState::Idle,
            reason: None,
            part_path: root.join(format!("{prefix}.part")),
            index_path: root.join(format!("{prefix}.idx")),
            destination_root: root,
            file: None,
            received: BTreeSet::new(),
        })
    }

    /// Evaluate an incoming offer against the policy.
    ///
    /// Offers over the hard cap fail immediately; anything above the auto-download limit
    /// waits for [`Transfer::accept`]; everything else is accepted at once.
    pub fn receive_offer(&mut self) -> Result<TransferState, String> {
        self.guard(TransferAction::Receive)?;
        if let Err(error) = self.offer.validate(self.policy.max_file_bytes) {
            self.state = TransferState::Failed;
            self.reason = Some(error.clone());
            return Err(error);
        }
        self.state = if self.policy.auto_downloads(self.offer.size) {
            TransferState::Accepted
        } else {
            TransferState::Offered
        };
        Ok(self.state)
    }

    /// Explicitly approve an offer that is waiting for the user.
    pub fn accept(&mut self) -> Result<TransferState, String> {
        self.guard(TransferAction::Accept)?;
        self.check_offer()?;
        self.state = TransferState::Accepted;
        self.reason = None;
        Ok(self.state)
    }

    /// Decline an offer without creating any file.
    pub fn refuse(&mut self, reason: &str) -> Result<TransferState, String> {
        self.guard(TransferAction::Refuse)?;
        self.state = TransferState::Failed;
        self.reason = Some(reason.to_owned());
        Ok(self.state)
    }

    /// Open the part file for a freshly accepted offer.
    pub fn begin(&mut self) -> Result<ResumePlan, String> {
        self.guard(TransferAction::Begin)?;
        self.start_engine()
    }

    /// Continue a paused transfer, reusing every verified chunk on disk.
    pub fn resume(&mut self) -> Result<ResumePlan, String> {
        self.guard(TransferAction::Resume)?;
        self.start_engine()
    }

    /// Retry a failed transfer. A hash failure discards all chunks first, so a retry only
    /// restarts from bytes that were never declared good.
    pub fn retry(&mut self) -> Result<ResumePlan, String> {
        self.guard(TransferAction::Retry)?;
        self.check_offer()?;
        self.start_engine()
    }

    /// Stop receiving while keeping verified chunks for a later resume.
    pub fn pause(&mut self) -> Result<ResumePlan, String> {
        self.guard(TransferAction::Pause)?;
        let plan = self.plan();
        self.close_file()?;
        self.state = TransferState::Paused;
        Ok(plan)
    }

    /// Store one received chunk; `Ok(false)` means it was already verified.
    pub fn write_chunk(&mut self, index: u64, bytes: &[u8]) -> Result<bool, String> {
        self.guard(TransferAction::Chunk)?;
        let total = self.offer.chunk_count();
        if index >= total {
            return Err(format!(
                "chunk index {index} is outside the {total} offered chunks"
            ));
        }
        let expected = expected_chunk_len(self.offer.size, total, index)
            .ok_or_else(|| "chunk offset overflow".to_owned())?;
        if bytes.len() != expected {
            return Err(format!(
                "chunk {index} must be {expected} bytes, got {}",
                bytes.len()
            ));
        }
        if self.received.contains(&index) {
            return Ok(false);
        }
        let offset = chunk_offset(index).ok_or_else(|| "chunk offset overflow".to_owned())?;
        let file = self
            .file
            .as_mut()
            .ok_or("transfer has no open part file")?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        self.append_index(index)?;
        self.received.insert(index);
        Ok(true)
    }

    /// Verify the whole file and move it into place.
    pub fn finish(&mut self) -> Result<PathBuf, String> {
        self.guard(TransferAction::Finish)?;
        let total = self.offer.chunk_count();
        if self.received.len() as u64 != total {
            return Err(format!(
                "transfer is missing {} chunks (next expected {})",
                total - self.received.len() as u64,
                self.next_index()
            ));
        }
        let mut file = self
            .file
            .take()
            .ok_or_else(|| "transfer has no open part file".to_owned())?;
        let hash = hash_file(&mut file)?;
        drop(file);
        if hash != self.offer.blake3_hash {
            self.discard_all("file hash does not match the offer");
            return Err("file hash does not match the offer".into());
        }
        let destination = self.destination_root.join(final_name(&self.offer));
        fs::rename(&self.part_path, &destination)
            .map_err(|error| error.to_string())?;
        let _ = fs::remove_file(&self.index_path);
        self.received.clear();
        self.state = TransferState::Completed;
        self.reason = None;
        Ok(destination)
    }

    /// Abort and remove every file this transfer created.
    pub fn cancel(&mut self) -> Result<(), String> {
        self.guard(TransferAction::Cancel)?;
        self.close_file()?;
        let _ = fs::remove_file(&self.part_path);
        let _ = fs::remove_file(&self.index_path);
        self.received.clear();
        self.state = TransferState::Cancelled;
        self.reason = Some("cancelled".into());
        Ok(())
    }

    #[must_use]
    pub fn state(&self) -> TransferState {
        self.state
    }

    #[must_use]
    pub fn offer(&self) -> &FileOffer {
        &self.offer
    }

    #[must_use]
    pub fn policy(&self) -> TransferPolicy {
        self.policy
    }

    /// Why the transfer failed, was refused or was cancelled.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    #[must_use]
    pub fn part_path(&self) -> &Path {
        &self.part_path
    }

    #[must_use]
    pub fn received_chunks(&self) -> u64 {
        self.received.len() as u64
    }

    /// Highest chunk index with no gap below it, if any chunk is verified.
    #[must_use]
    pub fn last_verified_chunk(&self) -> Option<u64> {
        let mut expected = 0_u64;
        let mut last = None;
        for index in &self.received {
            if *index != expected {
                break;
            }
            last = Some(*index);
            expected += 1;
        }
        last
    }

    /// First chunk index the sender still owes us.
    #[must_use]
    pub fn next_index(&self) -> u64 {
        let total = self.offer.chunk_count();
        (0..total)
            .find(|index| !self.received.contains(index))
            .unwrap_or(total)
    }

    /// The missing chunk indices, capped at `limit` so messages stay bounded.
    #[must_use]
    pub fn missing_chunks(&self, limit: usize) -> Vec<u64> {
        let total = self.offer.chunk_count();
        (0..total)
            .filter(|index| !self.received.contains(index))
            .take(limit.max(1))
            .collect()
    }

    #[must_use]
    pub fn progress(&self) -> Progress {
        let total_chunks = self.offer.chunk_count();
        let total_bytes = self.offer.size;
        let received_bytes = self
            .received
            .iter()
            .filter_map(|index| expected_chunk_len(total_bytes, total_chunks, *index))
            .sum::<usize>() as u64;
        let percent = if total_bytes == 0 || self.received.len() as u64 == total_chunks {
            100
        } else {
            received_bytes.saturating_mul(100) / total_bytes
        };
        Progress {
            received_chunks: self.received.len() as u64,
            total_chunks,
            received_bytes,
            total_bytes,
            percent: percent.min(100) as u32,
        }
    }

    #[must_use]
    pub fn plan(&self) -> ResumePlan {
        let total_chunks = self.offer.chunk_count();
        let received_chunks = self.received.len() as u64;
        ResumePlan {
            next_index: self.next_index(),
            received_chunks,
            total_chunks,
            missing_chunks: total_chunks.saturating_sub(received_chunks),
            progress: self.progress(),
        }
    }

    fn guard(&self, action: TransferAction) -> Result<(), String> {
        if self.state.allows(action) {
            return Ok(());
        }
        Err(format!(
            "{} is not allowed in state {}",
            action.as_str(),
            self.state.as_str()
        ))
    }

    /// An offer that no longer fits the policy must never reach the disk again.
    fn check_offer(&mut self) -> Result<(), String> {
        if let Err(error) = self.offer.validate(self.policy.max_file_bytes) {
            self.reason = Some(error.clone());
            return Err(error);
        }
        Ok(())
    }

    fn start_engine(&mut self) -> Result<ResumePlan, String> {
        self.open_parts()?;
        self.state = TransferState::Transferring;
        Ok(self.plan())
    }

    /// Open (or re-open) the part file and reload the verified chunk index.
    fn open_parts(&mut self) -> Result<(), String> {
        let existed = self.part_path.exists();
        let mut file = Self::open_part(&self.part_path)?;
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        if length != self.offer.size {
            if existed {
                // A stale part file from another offer cannot be trusted.
                drop(file);
                let _ = fs::remove_file(&self.part_path);
                let _ = fs::remove_file(&self.index_path);
                self.received.clear();
                file = Self::open_part(&self.part_path)?;
            }
            file.set_len(self.offer.size)
                .map_err(|error| error.to_string())?;
        }
        self.file = Some(file);
        self.load_index()
    }

    fn open_part(path: &Path) -> Result<File, String> {
        OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open(path)
            .map_err(|error| error.to_string())
    }

    fn load_index(&mut self) -> Result<(), String> {
        self.received.clear();
        let total = self.offer.chunk_count();
        let Ok(bytes) = fs::read(&self.index_path) else {
            return Ok(());
        };
        if bytes.len() > total as usize * INDEX_RECORD_BYTES {
            return self.reset_index();
        }
        let mut restored = BTreeSet::new();
        for record in bytes.chunks_exact(INDEX_RECORD_BYTES) {
            let Ok(record) = <[u8; INDEX_RECORD_BYTES]>::try_from(record) else {
                return self.reset_index();
            };
            let index = u64::from_le_bytes(record);
            if index >= total {
                return self.reset_index();
            }
            restored.insert(index);
        }
        self.received = restored;
        Ok(())
    }

    fn append_index(&mut self, index: u64) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.index_path)
            .map_err(|error| error.to_string())?;
        file.write_all(&index.to_le_bytes())
            .and_then(|_| file.flush())
            .map_err(|error| error.to_string())
    }

    fn reset_index(&mut self) -> Result<(), String> {
        self.received.clear();
        fs::write(&self.index_path, []).map_err(|error| error.to_string())
    }

    fn close_file(&mut self) -> Result<(), String> {
        if let Some(mut file) = self.file.take() {
            file.flush().map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn discard_all(&mut self, reason: &str) {
        self.file = None;
        let _ = fs::remove_file(&self.part_path);
        let _ = fs::remove_file(&self.index_path);
        self.received.clear();
        self.state = TransferState::Failed;
        self.reason = Some(reason.to_owned());
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
        let Some(expected_len) = expected_chunk_len(self.offer.size, total, index) else {
            return Err("chunk offset overflow".into());
        };
        if bytes.len() != expected_len {
            return Err("chunk length does not match the offered file".into());
        }
        if self.received.contains(&index) {
            return Ok(false);
        }
        let offset = chunk_offset(index).ok_or_else(|| "chunk offset overflow".to_owned())?;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        self.file
            .write_all(bytes)
            .map_err(|error| error.to_string())?;
        self.received.insert(index);
        Ok(true)
    }

    pub fn finish(mut self) -> Result<PathBuf, String> {
        if self.received.len() as u64 != self.offer.chunk_count() {
            return Err("file transfer is missing chunks".into());
        }
        if hash_file(&mut self.file)? != self.offer.blake3_hash {
            return Err("file hash does not match the offer".into());
        }
        let destination = self.destination_root.join(final_name(&self.offer));
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

    fn offer_for(contents: &[u8], name: &str) -> FileOffer {
        FileOffer {
            id: Uuid::now_v7(),
            file_name: name.into(),
            size: contents.len() as u64,
            blake3_hash: *blake3::hash(contents).as_bytes(),
        }
    }

    /// A payload spanning several full chunks so gaps and bounds are observable.
    fn multi_chunk_payload(chunks: usize) -> Vec<u8> {
        (0..chunks)
            .flat_map(|chunk| {
                (0..MAX_FILE_CHUNK_BYTES).map(move |byte| (chunk as u32 + byte as u32) as u8)
            })
            .collect()
    }

    fn chunk_of(contents: &[u8], index: u64) -> &[u8] {
        let size = contents.len() as u64;
        let total = size.div_ceil(MAX_FILE_CHUNK_BYTES as u64);
        let offset = chunk_offset(index).unwrap() as usize;
        let len = expected_chunk_len(size, total, index).unwrap();
        &contents[offset..offset + len]
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

    #[test]
    fn state_machine_only_permits_documented_transitions() {
        use TransferAction::{
            Accept, Begin, Cancel, Chunk, Finish, Pause, Receive, Refuse, Resume, Retry,
        };
        use TransferState::{
            Accepted, Cancelled, Completed, Failed, Idle, Offered, Paused, Transferring,
        };
        assert!(Idle.allows(Receive));
        assert!(!Idle.allows(Finish));
        assert!(Offered.allows(Accept) && Offered.allows(Refuse) && Offered.allows(Cancel));
        assert!(!Offered.allows(Pause));
        assert!(Accepted.allows(Begin) && !Accepted.allows(Chunk));
        // A user may still back out of an auto-accepted offer before a byte arrives.
        assert!(Accepted.allows(Refuse));
        assert!(Transferring.allows(Chunk) && Transferring.allows(Pause));
        assert!(!Transferring.allows(Accept));
        assert!(Paused.allows(Resume) && !Paused.allows(Finish));
        assert!(Failed.allows(Retry) && !Failed.allows(Chunk));
        for terminal in [Completed, Cancelled] {
            assert!(terminal.is_terminal());
            for action in [
                Receive, Accept, Refuse, Begin, Pause, Resume, Retry, Chunk, Finish, Cancel,
            ] {
                assert!(
                    !terminal.allows(action),
                    "{terminal:?} must be frozen against {action:?}"
                );
            }
        }
        for state in [
            Idle,
            Offered,
            Accepted,
            Transferring,
            Paused,
            Completed,
            Failed,
            Cancelled,
        ] {
            assert_eq!(TransferState::parse(state.as_str()), Some(state));
        }
        assert_eq!(
            TransferState::parse("transferring"),
            Some(TransferState::Transferring)
        );
        assert_eq!(TransferState::parse("whatever"), None);
        assert_eq!(TransferAction::Chunk.as_str(), "write_chunk");
    }

    #[test]
    fn auto_download_limit_and_max_size_decide_how_an_offer_lands() {
        let root = temp_dir();
        let policy = TransferPolicy::new(1_000_000, 512);
        assert_eq!(policy.auto_download_limit_bytes, 512);
        // The auto limit can never sit above the hard cap.
        assert_eq!(
            TransferPolicy::new(100, 9_000).auto_download_limit_bytes,
            100
        );

        let tiny = offer_for(b"under the limit", "tiny.bin");
        let mut automatic = Transfer::incoming(&root, tiny.clone(), policy).unwrap();
        assert_eq!(automatic.state(), TransferState::Idle);
        // Nothing touches the disk before an offer is even evaluated.
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        assert!(policy.auto_downloads(tiny.size));
        assert_eq!(automatic.receive_offer().unwrap(), TransferState::Accepted);
        automatic.begin().unwrap();
        automatic.cancel().unwrap();

        let asked = offer_for(&multi_chunk_payload(1), "big.bin");
        let mut waiting = Transfer::incoming(&root, asked.clone(), policy).unwrap();
        assert!(!policy.auto_downloads(asked.size));
        assert_eq!(waiting.receive_offer().unwrap(), TransferState::Offered);
        assert!(waiting.begin().is_err(), "an unaccepted offer cannot start");
        assert_eq!(waiting.accept().unwrap(), TransferState::Accepted);
        assert_eq!(waiting.plan().next_index, 0);
        waiting.cancel().unwrap();

        let greedy = offer_for(&multi_chunk_payload(1), "greedy.bin");
        let mut refused = Transfer::incoming(
            &root,
            greedy.clone(),
            TransferPolicy::new(greedy.size - 1, 512),
        )
        .unwrap();
        assert!(refused.receive_offer().is_err());
        assert_eq!(refused.state(), TransferState::Failed);
        assert!(refused.reason().unwrap().contains("size limit"));
        assert!(refused.begin().is_err());
        assert!(refused.accept().is_err());
        // Retrying must not smuggle an over-cap offer onto the disk either.
        assert!(refused.retry().is_err());
        assert!(!refused.part_path().exists());

        let mut hostile = Transfer::incoming(
            &root,
            offer_for(b"data", "../evil.bin"),
            TransferPolicy::default(),
        )
        .unwrap();
        assert!(hostile.receive_offer().is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pause_and_resume_reuse_verified_chunks_across_sessions() {
        let root = temp_dir();
        let contents = multi_chunk_payload(3);
        let offer = offer_for(&contents, "report.bin");
        assert_eq!(offer.chunk_count(), 3);
        let mut transfer =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        transfer.receive_offer().unwrap();
        let plan = transfer.begin().unwrap();
        assert_eq!(plan.total_chunks, 3);
        assert_eq!(plan.next_index, 0);
        for index in [0, 2] {
            transfer
                .write_chunk(index, chunk_of(&contents, index))
                .unwrap();
        }
        assert_eq!(transfer.received_chunks(), 2);
        assert_eq!(transfer.last_verified_chunk(), Some(0));
        assert_eq!(transfer.next_index(), 1);
        assert_eq!(transfer.missing_chunks(8), vec![1]);
        assert_eq!(transfer.progress().percent, 66);
        let paused = transfer.pause().unwrap();
        assert_eq!(transfer.state(), TransferState::Paused);
        assert_eq!(paused.next_index, 1);
        assert!(transfer.part_path().exists());
        // Resuming the same instance continues from the last verified chunk.
        assert_eq!(transfer.resume().unwrap().received_chunks, 2);

        // Re-opening like a fresh process: the sidecar index restores the verified
        // chunks so the sender only resends the gap.
        let mut revived =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        revived.receive_offer().unwrap();
        let reopened = revived.begin().unwrap();
        assert_eq!(revived.state(), TransferState::Transferring);
        assert_eq!(
            reopened.received_chunks, 2,
            "verified chunks survive a restart"
        );
        assert_eq!(reopened.next_index, 1);
        assert!(
            !revived.write_chunk(0, chunk_of(&contents, 0)).unwrap(),
            "already verified chunks are not written twice"
        );
        revived.write_chunk(1, chunk_of(&contents, 1)).unwrap();
        assert!(
            revived.resume().is_err(),
            "only a paused transfer may resume in place"
        );
        let path = revived.finish().unwrap();
        assert_eq!(revived.state(), TransferState::Completed);
        assert_eq!(fs::read(&path).unwrap(), contents);
        assert!(!revived.part_path().exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_chunk_indices_beyond_the_offer_and_wrong_lengths() {
        let root = temp_dir();
        let contents = multi_chunk_payload(2);
        let offer = offer_for(&contents, "bounded.bin");
        let total = offer.chunk_count();
        let mut transfer =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        transfer.receive_offer().unwrap();
        transfer.begin().unwrap();
        assert!(transfer.write_chunk(2, b"one chunk too many").is_err());
        assert!(transfer.write_chunk(u64::MAX, b"x").is_err());
        assert!(transfer
            .write_chunk(0, &contents[..MAX_FILE_CHUNK_BYTES - 1])
            .is_err());
        assert!(transfer
            .write_chunk(1, vec![0_u8; MAX_FILE_CHUNK_BYTES + 1].as_slice())
            .is_err());
        assert_eq!(transfer.received_chunks(), 0, "refused writes store nothing");
        assert!(transfer.write_chunk(0, chunk_of(&contents, 0)).unwrap());
        assert!(!transfer.write_chunk(0, chunk_of(&contents, 0)).unwrap());
        assert!(transfer.write_chunk(1, chunk_of(&contents, 1)).unwrap());
        let path = transfer.finish().unwrap();
        assert_eq!(fs::read(&path).unwrap(), contents);

        assert_eq!(
            expected_chunk_len(offer.size, total, 0),
            Some(MAX_FILE_CHUNK_BYTES)
        );
        assert_eq!(
            expected_chunk_len(offer.size, total, 1),
            Some(MAX_FILE_CHUNK_BYTES)
        );
        assert_eq!(expected_chunk_len(offer.size, total, 2), None);
        assert_eq!(expected_chunk_len(0, 0, 0), None);
        assert_eq!(expected_chunk_len(offer.size, total, u64::MAX), None);
        assert_eq!(chunk_offset(u64::MAX), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hash_mismatch_discards_everything_and_a_retry_starts_clean() {
        let root = temp_dir();
        let contents = multi_chunk_payload(2);
        let mut offer = offer_for(&contents, "hashed.bin");
        offer.blake3_hash = *blake3::hash(b"something else").as_bytes();
        let mut transfer =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        transfer.receive_offer().unwrap();
        transfer.begin().unwrap();
        for index in 0..offer.chunk_count() {
            transfer
                .write_chunk(index, chunk_of(&contents, index))
                .unwrap();
        }
        assert!(transfer.finish().is_err());
        assert_eq!(transfer.state(), TransferState::Failed);
        assert!(transfer.reason().unwrap().contains("hash"));
        assert!(!transfer.part_path().exists());
        assert_eq!(transfer.received_chunks(), 0);
        // A hash-bad transfer may be retried, but nothing suspect is reused.
        assert_eq!(transfer.plan().missing_chunks, 2);

        let good = FileOffer {
            id: offer.id,
            ..offer_for(&contents, "hashed.bin")
        };
        let mut retry =
            Transfer::incoming(&root, good.clone(), TransferPolicy::default()).unwrap();
        retry.receive_offer().unwrap();
        let plan = retry.begin().unwrap();
        assert_eq!(plan.received_chunks, 0, "discarded bytes do not come back");
        assert!(retry.finish().is_err(), "still missing chunks");
        for index in 0..good.chunk_count() {
            retry
                .write_chunk(index, chunk_of(&contents, index))
                .unwrap();
        }
        let path = retry.finish().unwrap();
        assert_eq!(fs::read(&path).unwrap(), contents);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancel_and_guards_leave_nothing_behind() {
        let root = temp_dir();
        let contents = multi_chunk_payload(2);
        let offer = offer_for(&contents, "cancelled.bin");
        let mut transfer =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        transfer.receive_offer().unwrap();
        transfer.begin().unwrap();
        transfer.write_chunk(0, chunk_of(&contents, 0)).unwrap();
        assert!(transfer.finish().is_err(), "a gap must not finish");
        transfer.cancel().unwrap();
        assert_eq!(transfer.state(), TransferState::Cancelled);
        assert!(!transfer.part_path().exists());
        assert!(transfer.cancel().is_err());
        assert!(transfer.write_chunk(0, chunk_of(&contents, 0)).is_err());
        assert!(transfer.refuse("too late").is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

        let mut idle =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        assert!(idle.pause().is_err(), "nothing to pause before a begin");
        assert!(idle.finish().is_err());
        idle.receive_offer().unwrap();
        idle.refuse("user said no").unwrap();
        assert_eq!(idle.state(), TransferState::Failed);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn progress_reports_bytes_chunks_and_percent() {
        let root = temp_dir();
        let contents = multi_chunk_payload(4);
        let offer = offer_for(&contents, "progress.bin");
        let mut transfer =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        transfer.receive_offer().unwrap();
        transfer.begin().unwrap();
        assert_eq!(
            transfer.progress(),
            Progress {
                received_chunks: 0,
                total_chunks: 4,
                received_bytes: 0,
                total_bytes: contents.len() as u64,
                percent: 0,
            }
        );
        transfer.write_chunk(0, chunk_of(&contents, 0)).unwrap();
        let step = transfer.progress();
        assert_eq!(step.received_chunks, 1);
        assert_eq!(step.total_chunks, 4);
        assert_eq!(step.received_bytes, MAX_FILE_CHUNK_BYTES as u64);
        assert_eq!(step.total_bytes, contents.len() as u64);
        assert_eq!(step.percent, 25);
        for index in 1..4 {
            transfer
                .write_chunk(index, chunk_of(&contents, index))
                .unwrap();
        }
        assert_eq!(transfer.progress().percent, 100);
        assert_eq!(transfer.plan().missing_chunks, 0);
        assert_eq!(transfer.next_index(), 4);
        assert_eq!(transfer.last_verified_chunk(), Some(3));
        assert!(transfer.missing_chunks(8).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_resume_index_self_heals() {
        let root = temp_dir();
        let contents = multi_chunk_payload(2);
        let offer = offer_for(&contents, "index.bin");
        let mut transfer =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        transfer.receive_offer().unwrap();
        transfer.begin().unwrap();
        transfer.write_chunk(0, chunk_of(&contents, 0)).unwrap();
        transfer.pause().unwrap();
        // A peer (or a torn write) left an index record outside the offer.
        let index_path = root.join(format!(".alverchat-{}-resume.idx", offer.id));
        fs::write(&index_path, u64::MAX.to_le_bytes()).unwrap();

        let mut revived =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        revived.receive_offer().unwrap();
        revived.begin().unwrap();
        assert_eq!(revived.received_chunks(), 0, "bad index records are dropped");
        assert_eq!(revived.next_index(), 0);
        revived.write_chunk(0, chunk_of(&contents, 0)).unwrap();
        revived.write_chunk(1, chunk_of(&contents, 1)).unwrap();
        assert!(revived.finish().is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_stale_part_file_from_another_offer_is_not_trusted() {
        let root = temp_dir();
        let contents = multi_chunk_payload(2);
        let offer = offer_for(&contents, "shared.bin");
        let mut first =
            Transfer::incoming(&root, offer.clone(), TransferPolicy::default()).unwrap();
        first.receive_offer().unwrap();
        first.begin().unwrap();
        first.write_chunk(0, chunk_of(&contents, 0)).unwrap();
        assert!(root
            .join(format!(".alverchat-{}-resume.idx", offer.id))
            .exists());

        // Same id, different declared size: the leftover bytes cannot be reused.
        let mut changed = offer.clone();
        changed.size += 7;
        let mut second =
            Transfer::incoming(&root, changed.clone(), TransferPolicy::default()).unwrap();
        second.receive_offer().unwrap();
        let plan = second.begin().unwrap();
        assert_eq!(plan.received_chunks, 0, "a mismatched part file restarts");
        assert_eq!(second.plan().total_chunks, 3);
        assert_eq!(
            fs::metadata(second.part_path()).unwrap().len(),
            changed.size
        );
        second.cancel().unwrap();
        assert!(!second.part_path().exists());
        let _ = first.cancel();
        fs::remove_dir_all(root).unwrap();
    }
}
