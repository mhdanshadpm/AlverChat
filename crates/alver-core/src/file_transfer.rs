//! Chunked, resumable, hash-verified file transfer over an authenticated peer session.
//!
//! A `file_offer` is only a notification: it carries no bytes. The receiver owns the data
//! connection — it opens a session, sends `file_accept` naming the first chunk it still
//! needs, and the sender then streams `file_chunk` frames until the receiver answers
//! `file_ack { complete: true }` or a `file_cancel` ends the run. Because the puller owns
//! the connection, a pause, a crash or a reconnect costs nothing: the verified chunks live
//! in the part file beside the download and the next `file_accept` starts from the first
//! index still owed. Both drivers re-check the local transfer state between chunks, so a
//! click in the UI stops a running transfer without any shared registry.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    files::{chunk_offset, expected_chunk_len, Transfer, TransferState},
    payloads::{kind, FileAcceptPayload, FileAckPayload, FileCancelPayload, FileChunkPayload},
    protocol::MAX_FILE_CHUNK_BYTES,
    transport::SecureStream,
    Envelope, Hlc, PROTOCOL_VERSION,
};

/// Chunks pushed before the sender waits for the peer's acknowledgements.
const CHUNK_WINDOW: u64 = 4;

/// `file_cancel` reason that means "stop here, we will pick this up later" instead of
/// "abort and throw the received bytes away".
pub const PAUSE_REASON: &str = "paused";
/// `file_cancel` reason sent when either user cancels.
pub const CANCEL_REASON: &str = "cancelled";

/// Who is talking to whom on one transfer session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferAddress {
    /// Signed peer id of this side.
    pub from: String,
    /// Signed peer id of the other side.
    pub to: String,
}

/// What the local user decided while a driver was running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Keep going.
    Proceed,
    /// Stop, but keep every verified chunk for a later resume.
    Pause,
    /// Stop and discard what is not yet confirmed.
    Cancel,
}

/// A transfer's state as one driver last saw it, ready to persist and publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferUpdate {
    pub state: TransferState,
    /// First chunk index the peer still owes (sender) or we still owe (receiver).
    pub next_index: u64,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    /// Whole percentage, 0..=100.
    pub percent: u32,
    /// Where the finished file landed, for a completed receive.
    pub path: Option<PathBuf>,
    pub reason: Option<String>,
}

impl TransferUpdate {
    fn sending(
        next_index: u64,
        total_chunks: u64,
        total_bytes: u64,
        reason: Option<String>,
    ) -> Self {
        let transferred = next_index
            .saturating_mul(MAX_FILE_CHUNK_BYTES as u64)
            .min(total_bytes);
        Self {
            state: if next_index >= total_chunks {
                TransferState::Completed
            } else {
                TransferState::Transferring
            },
            next_index,
            transferred_bytes: transferred,
            total_bytes,
            percent: percent(transferred, total_bytes),
            path: None,
            reason,
        }
    }

    fn receiving(transfer: &Transfer, reason: Option<String>) -> Self {
        let progress = transfer.progress();
        Self {
            state: transfer.state(),
            next_index: transfer.next_index(),
            transferred_bytes: progress.received_bytes,
            total_bytes: progress.total_bytes,
            percent: progress.percent,
            path: None,
            reason,
        }
    }

    fn with_state(mut self, state: TransferState) -> Self {
        self.state = state;
        self
    }
}

fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 100;
    }
    ((done.saturating_mul(100) / total).min(100)) as u32
}

/// Builds one file-transfer envelope; the caller owns the clock it is stamped with.
pub fn frame<T: Serialize>(
    address: &TransferAddress,
    kind_name: &str,
    payload: &T,
    clock: Hlc,
) -> Result<Envelope<Value>, String> {
    Ok(Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: kind_name.to_owned(),
        from: address.from.clone(),
        to: address.to.clone(),
        hlc: clock,
        payload: serde_json::to_value(payload).map_err(|error| error.to_string())?,
    })
}

/// Sends one frame and advances the session's hybrid clock.
fn send<T: Serialize, S: Read + Write>(
    session: &mut SecureStream<S>,
    address: &TransferAddress,
    kind_name: &str,
    payload: &T,
    clock: &mut Hlc,
) -> Result<(), String> {
    let envelope = frame(address, kind_name, payload, *clock)?;
    clock.logical = clock.logical.saturating_add(1);
    session.send(&envelope)
}

/// One inbound frame, checked against the authenticated peer and this transfer.
fn receive<S: Read + Write>(
    session: &mut SecureStream<S>,
    own_id: &str,
    transfer_id: Uuid,
) -> Result<(String, Value), String> {
    let envelope: Envelope<Value> = session.receive()?;
    let claimed: Option<Uuid> = envelope
        .payload
        .get("transfer_id")
        .and_then(|value| serde_json::from_value::<String>(value.clone()).ok())
        .and_then(|raw| Uuid::parse_str(&raw).ok());
    if envelope.v != PROTOCOL_VERSION
        || envelope.from != session.remote_identity().peer_id
        || envelope.to != own_id
        || claimed != Some(transfer_id)
    {
        return Err("peer sent a file transfer frame that is not ours".into());
    }
    Ok((envelope.kind, envelope.payload))
}

fn payload<T: DeserializeOwned>(kind_name: &str, value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|error| format!("{kind_name} frame is malformed: {error}"))
}

/// Everything `push_chunks` needs about one outgoing file.
#[derive(Clone, Debug)]
pub struct SendRequest<'a> {
    pub address: &'a TransferAddress,
    pub transfer_id: Uuid,
    /// The local copy of the bytes; its length is the size that was offered.
    pub source: &'a Path,
    pub clock_ms: u64,
}

/// Streams `request.source` over an established session until the peer confirms it.
///
/// The peer's `file_accept` is read first and decides where the stream starts: only the
/// receiver knows which chunks survived a pause, a crash or a reconnect. `report` sees
/// every batch boundary, which is how pause and cancel reach a running sender.
pub fn push_chunks<S, R>(
    session: &mut SecureStream<S>,
    request: &SendRequest,
    report: R,
) -> Result<TransferUpdate, String>
where
    S: Read + Write,
    R: FnMut(&TransferUpdate) -> Gate,
{
    let (kind_name, body) = receive(session, &request.address.from, request.transfer_id)?;
    if kind_name != kind::FILE_ACCEPT {
        return Err(format!(
            "peer sent {kind_name} instead of accepting the file"
        ));
    }
    let accept: FileAcceptPayload = payload(&kind_name, body)?;
    push_accepted(session, request, &accept, report)
}

/// Streams `request.source` for a caller that already dispatched on the `file_accept`.
///
/// The desktop session loop reads one envelope at a time, so the resume position the
/// receiver asked for is in hand before the chunk stream starts.
pub fn push_accepted<S, R>(
    session: &mut SecureStream<S>,
    request: &SendRequest,
    accept: &FileAcceptPayload,
    mut report: R,
) -> Result<TransferUpdate, String>
where
    S: Read + Write,
    R: FnMut(&TransferUpdate) -> Gate,
{
    let total_bytes = fs::metadata(request.source)
        .map_err(|error| format!("cannot read the file to send: {error}"))?
        .len();
    if total_bytes == 0 {
        return Err("file to send is empty".into());
    }
    let total_chunks = total_bytes.div_ceil(MAX_FILE_CHUNK_BYTES as u64);
    if accept.resume_from >= total_chunks {
        return Err("the peer already holds every chunk of this file".into());
    }
    let mut source = File::open(request.source).map_err(|error| error.to_string())?;
    let mut clock = Hlc {
        physical_ms: request.clock_ms,
        logical: 0,
    };
    let mut next_index = accept.resume_from;
    let mut update = TransferUpdate::sending(next_index, total_chunks, total_bytes, None);

    while next_index < total_chunks {
        match report(&update) {
            Gate::Proceed => {}
            Gate::Pause => return Ok(update.with_state(TransferState::Paused)),
            Gate::Cancel => {
                send(
                    session,
                    request.address,
                    kind::FILE_CANCEL,
                    &FileCancelPayload {
                        transfer_id: request.transfer_id,
                        reason: Some(CANCEL_REASON.into()),
                    },
                    &mut clock,
                )?;
                return Ok(update.with_state(TransferState::Cancelled));
            }
        }
        let batch_start = next_index;
        let batch_end = total_chunks.min(next_index + CHUNK_WINDOW);
        while next_index < batch_end {
            let offset = chunk_offset(next_index).ok_or("chunk offset overflow")?;
            let len = expected_chunk_len(total_bytes, total_chunks, next_index)
                .ok_or("chunk index is outside the file")?;
            source
                .seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
            let mut bytes = vec![0_u8; len];
            source
                .read_exact(&mut bytes)
                .map_err(|error| format!("file shrank while sending: {error}"))?;
            send(
                session,
                request.address,
                kind::FILE_CHUNK,
                &FileChunkPayload {
                    transfer_id: request.transfer_id,
                    index: next_index,
                    bytes,
                    last: next_index + 1 == total_chunks,
                },
                &mut clock,
            )?;
            next_index += 1;
        }

        let mut held = None;
        for _ in batch_start..next_index {
            let (kind_name, body) = receive(session, &request.address.from, request.transfer_id)?;
            match kind_name.as_str() {
                kind::FILE_ACK => {
                    let ack: FileAckPayload = payload(&kind_name, body)?;
                    if ack.contiguous_index >= total_chunks {
                        return Err("peer acknowledged a chunk index beyond the file".into());
                    }
                    if ack.complete {
                        return Ok(completed(total_chunks, total_bytes));
                    }
                    held = Some(held.map_or(ack.contiguous_index, |best: u64| {
                        best.max(ack.contiguous_index)
                    }));
                }
                kind::FILE_CANCEL => {
                    return cancel_outcome(
                        &kind_name,
                        body,
                        held.map_or(batch_start, |index| index + 1),
                        total_chunks,
                        total_bytes,
                    );
                }
                other => return Err(format!("peer sent an unexpected {other} frame")),
            }
        }
        let held = held.ok_or("peer acknowledged nothing")?;
        if held + 1 != next_index {
            return Err(format!(
                "peer only holds {} of the {} chunks sent",
                held + 1,
                next_index
            ));
        }
        if next_index == total_chunks {
            // Nothing is a delivered file until the peer has re-hashed it: the final
            // chunk's ack only says "stored", so one more frame carries the verdict.
            let (kind_name, body) = receive(session, &request.address.from, request.transfer_id)?;
            return match kind_name.as_str() {
                kind::FILE_ACK => {
                    let ack: FileAckPayload = payload(&kind_name, body)?;
                    if ack.complete {
                        Ok(completed(total_chunks, total_bytes))
                    } else {
                        Err("peer returned a final acknowledgement that was not complete".into())
                    }
                }
                kind::FILE_CANCEL => {
                    cancel_outcome(&kind_name, body, total_chunks, total_chunks, total_bytes)
                }
                other => Err(format!("peer sent an unexpected {other} frame")),
            };
        }
        update = TransferUpdate::sending(next_index, total_chunks, total_bytes, None);
    }
    Ok(update)
}

fn completed(total_chunks: u64, total_bytes: u64) -> TransferUpdate {
    TransferUpdate::sending(total_chunks, total_chunks, total_bytes, None)
        .with_state(TransferState::Completed)
}

/// Turns a received `file_cancel` into the state the sender is left in.
fn cancel_outcome(
    kind_name: &str,
    body: Value,
    confirmed: u64,
    total_chunks: u64,
    total_bytes: u64,
) -> Result<TransferUpdate, String> {
    let cancel: FileCancelPayload = payload(kind_name, body)?;
    let reason = cancel
        .reason
        .unwrap_or_else(|| "peer cancelled the transfer".into());
    let state = match reason.as_str() {
        PAUSE_REASON => TransferState::Paused,
        CANCEL_REASON => TransferState::Cancelled,
        _ => TransferState::Failed,
    };
    Ok(TransferUpdate::sending(
        confirmed,
        total_chunks,
        total_bytes,
        if state == TransferState::Failed {
            Some(reason)
        } else {
            None
        },
    )
    .with_state(state))
}

/// Everything `receive_chunks` needs about one incoming transfer.
#[derive(Clone, Debug)]
pub struct ReceiveRequest<'a> {
    pub address: &'a TransferAddress,
    pub clock_ms: u64,
}

/// Requests and stores the chunks a begun [`Transfer`] still needs.
///
/// The transfer must already be in the `Transferring` state, and must still owe at least
/// one chunk; a part file that already holds everything is finished by the caller instead.
pub fn receive_chunks<S, R>(
    session: &mut SecureStream<S>,
    transfer: &mut Transfer,
    request: &ReceiveRequest,
    mut report: R,
) -> Result<TransferUpdate, String>
where
    S: Read + Write,
    R: FnMut(&TransferUpdate) -> Gate,
{
    if transfer.state() != TransferState::Transferring {
        return Err("transfer must be receiving before chunks are requested".into());
    }
    let offer = transfer.offer().clone();
    let total_chunks = offer.chunk_count();
    let plan = transfer.plan();
    if plan.next_index >= total_chunks {
        return Err("transfer already holds every offered chunk".into());
    }
    let mut clock = Hlc {
        physical_ms: request.clock_ms,
        logical: 0,
    };
    send(
        session,
        request.address,
        kind::FILE_ACCEPT,
        &FileAcceptPayload {
            transfer_id: offer.id,
            resume_from: plan.next_index,
        },
        &mut clock,
    )?;
    let mut update = TransferUpdate::receiving(transfer, None);

    loop {
        match report(&update) {
            Gate::Proceed => {}
            Gate::Pause => {
                send(
                    session,
                    request.address,
                    kind::FILE_CANCEL,
                    &FileCancelPayload {
                        transfer_id: offer.id,
                        reason: Some(PAUSE_REASON.into()),
                    },
                    &mut clock,
                )?;
                return paused(transfer);
            }
            Gate::Cancel => {
                send(
                    session,
                    request.address,
                    kind::FILE_CANCEL,
                    &FileCancelPayload {
                        transfer_id: offer.id,
                        reason: Some(CANCEL_REASON.into()),
                    },
                    &mut clock,
                )?;
                transfer.cancel()?;
                return Ok(
                    TransferUpdate::receiving(transfer, Some(CANCEL_REASON.into()))
                        .with_state(TransferState::Cancelled),
                );
            }
        }
        let (kind_name, body) = receive(session, &request.address.from, offer.id)?;
        match kind_name.as_str() {
            kind::FILE_CHUNK => {
                let chunk: FileChunkPayload = payload(&kind_name, body)?;
                transfer
                    .write_chunk(chunk.index, &chunk.bytes)
                    .map_err(|error| format!("cannot store chunk {}: {error}", chunk.index))?;
                let complete = transfer.received_chunks() == total_chunks;
                let contiguous_index = transfer
                    .last_verified_chunk()
                    .ok_or("no chunk is contiguous from the start of the file")?;
                send(
                    session,
                    request.address,
                    kind::FILE_ACK,
                    &FileAckPayload {
                        transfer_id: offer.id,
                        contiguous_index,
                        // An ack only says "stored"; `finish` carries the verdict.
                        complete: false,
                    },
                    &mut clock,
                )?;
                if complete {
                    return finish(session, request, transfer, &mut clock);
                }
                update = TransferUpdate::receiving(transfer, None);
            }
            kind::FILE_CANCEL => {
                let cancel: FileCancelPayload = payload(&kind_name, body)?;
                let reason = cancel
                    .reason
                    .unwrap_or_else(|| "sender cancelled the transfer".into());
                if reason == PAUSE_REASON {
                    return paused(transfer);
                }
                let _ = transfer.cancel();
                return Err(reason);
            }
            other => return Err(format!("peer sent an unexpected {other} frame")),
        }
    }
}

/// Stops a receiving driver while keeping every verified chunk for a later resume.
fn paused(transfer: &mut Transfer) -> Result<TransferUpdate, String> {
    transfer.pause()?;
    Ok(
        TransferUpdate::receiving(transfer, Some(PAUSE_REASON.into()))
            .with_state(TransferState::Paused),
    )
}

/// Verifies the whole file, then carries the verdict back to the sender.
fn finish<S: Read + Write>(
    session: &mut SecureStream<S>,
    request: &ReceiveRequest,
    transfer: &mut Transfer,
    clock: &mut Hlc,
) -> Result<TransferUpdate, String> {
    let transfer_id = transfer.offer().id;
    let total_bytes = transfer.offer().size;
    let total_chunks = transfer.offer().chunk_count();
    let path = match transfer.finish() {
        Ok(path) => path,
        Err(error) => {
            // The sender is still waiting on this session's acks; tell it the bytes were
            // thrown away so it does not sit there resending a file we already rejected.
            let _ = send(
                session,
                request.address,
                kind::FILE_CANCEL,
                &FileCancelPayload {
                    transfer_id,
                    reason: Some(error.clone()),
                },
                clock,
            );
            return Err(error);
        }
    };
    send(
        session,
        request.address,
        kind::FILE_ACK,
        &FileAckPayload {
            transfer_id,
            contiguous_index: total_chunks - 1,
            complete: true,
        },
        clock,
    )?;
    Ok(TransferUpdate {
        state: TransferState::Completed,
        next_index: total_chunks,
        transferred_bytes: total_bytes,
        total_bytes,
        percent: 100,
        path: Some(path),
        reason: None,
    })
}
