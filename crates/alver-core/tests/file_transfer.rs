//! End-to-end file transfer between two real Noise sessions over loopback TCP.
//!
//! These tests exercise the whole chunk protocol — offer, accept, base64 chunk frames,
//! per-chunk acknowledgements, the final verdict, pause, resume and cancel — against the
//! same `files::Transfer` engine the desktop app uses, so a regression in the framing or in
//! the resume index fails here rather than on a LAN.
//!
//! A test never leaves one side writing while the other has stopped reading, so a small
//! loopback socket buffer cannot turn a pause into a hang.

use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    thread::{self, JoinHandle},
};

use alver_core::{
    file_transfer::{
        frame, push_accepted, push_chunks, receive_chunks, Gate, ReceiveRequest, SendRequest,
        TransferAddress, TransferUpdate, CANCEL_REASON, PAUSE_REASON,
    },
    files::{expected_chunk_len, FileOffer, Transfer, TransferPolicy, TransferState},
    identity::Identity,
    payloads::{kind, FileAcceptPayload, FileChunkPayload},
    protocol::MAX_FILE_CHUNK_BYTES,
    transport::{initiator_handshake, responder_handshake, RemoteIdentity, SecureStream},
    Envelope, Hlc,
};
use serde_json::Value;
use uuid::Uuid;

type Stream = SecureStream<TcpStream>;

fn temp_dir(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("alverchat-transfer-{tag}-{}", Uuid::now_v7()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let client = TcpStream::connect(address).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

fn remote(identity: &Identity) -> RemoteIdentity {
    RemoteIdentity {
        peer_id: identity.peer_id(),
        public_key: identity.verifying_key().to_bytes(),
    }
}

fn between(from: &Identity, to: &Identity) -> TransferAddress {
    TransferAddress {
        from: from.peer_id(),
        to: to.peer_id(),
    }
}

/// A handshaked (initiator, responder) pair of authenticated streams on a new connection.
fn streams(alice: Arc<Identity>, bob: Arc<Identity>) -> (Stream, Stream) {
    let (alice_socket, bob_socket) = tcp_pair();
    let expected_bob = remote(&bob);
    let responder = thread::spawn(move || responder_handshake(bob_socket, &bob));
    let initiator = initiator_handshake(alice_socket, &alice, &expected_bob).unwrap();
    (initiator, responder.join().unwrap().unwrap())
}

/// Six chunks with a partial tail, so windowing, bounds and resumes are all visible.
fn six_chunk_payload() -> Vec<u8> {
    let len = MAX_FILE_CHUNK_BYTES * 5 + 999;
    (0..len).map(|index| (index % 251) as u8).collect()
}

/// One small chunk, so a side that stops reading cannot fill the loopback buffer.
fn one_chunk_payload() -> Vec<u8> {
    vec![0x5a_u8; MAX_FILE_CHUNK_BYTES / 4]
}

fn write_source(dir: &Path, contents: &[u8]) -> PathBuf {
    let path = dir.join("source.bin");
    fs::write(&path, contents).unwrap();
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

/// A receive-side transfer, already evaluated against the policy and opened.
fn begun(root: &Path, offer: FileOffer) -> Transfer {
    let mut transfer = Transfer::incoming(root, offer, TransferPolicy::default()).unwrap();
    transfer.receive_offer().unwrap();
    transfer.begin().unwrap();
    transfer
}

fn proceed(_update: &TransferUpdate) -> Gate {
    Gate::Proceed
}

fn receiving(address: &TransferAddress, clock_ms: u64) -> ReceiveRequest<'_> {
    ReceiveRequest { address, clock_ms }
}

fn sending<'a>(
    address: &'a TransferAddress,
    offer: &FileOffer,
    source: &'a Path,
    clock_ms: u64,
) -> SendRequest<'a> {
    SendRequest {
        address,
        transfer_id: offer.id,
        source,
        clock_ms,
    }
}

fn transfer_part_exists(root: &Path, transfer_id: Uuid) -> bool {
    fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains(&transfer_id.to_string())
        })
}

#[test]
fn a_six_chunk_file_arrives_hash_verified_on_the_other_side() {
    let alice = Arc::new(Identity::from_secret([40; 32]));
    let bob = Arc::new(Identity::from_secret([41; 32]));
    let root = temp_dir("happy");
    let contents = six_chunk_payload();
    let source = write_source(&root, &contents);
    let offer = offer_for(&contents, "quarterly report.bin");
    assert_eq!(offer.chunk_count(), 6);
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive = thread::spawn(move || {
        let result = receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 1_000),
            proceed,
        );
        (result, transfer.state())
    });

    let alice_address = between(&alice, &bob);
    let sent = push_chunks(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 1_001),
        proceed,
    )
    .unwrap();
    let (received, engine_state) = receive.join().unwrap();
    let received = received.unwrap();

    assert_eq!(sent.state, TransferState::Completed);
    assert_eq!(sent.next_index, 6);
    assert_eq!(sent.percent, 100);
    assert_eq!(received.state, TransferState::Completed);
    assert_eq!(engine_state, TransferState::Completed);
    assert_eq!(received.next_index, 6);
    assert_eq!(received.total_bytes, contents.len() as u64);
    let landed = received.path.expect("a verified file lands on disk");
    assert_eq!(fs::read(&landed).unwrap(), contents);
    assert_eq!(
        landed.file_name().unwrap().to_string_lossy(),
        alver_core::files::stored_name(offer.id, "quarterly report.bin")
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2, "source + landed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_sender_pause_keeps_the_verified_chunks_and_a_resume_finishes_the_file() {
    let alice = Arc::new(Identity::from_secret([42; 32]));
    let bob = Arc::new(Identity::from_secret([43; 32]));
    let root = temp_dir("pause");
    let contents = six_chunk_payload();
    let source = write_source(&root, &contents);
    let offer = offer_for(&contents, "resume.bin");

    // Session one: the sender stops once the receiver has confirmed four chunks. The
    // receiver is still reading, so nothing queues up behind the pause.
    let mut transfer = begun(&root, offer.clone());
    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive: JoinHandle<(Result<TransferUpdate, String>, u64)> = thread::spawn(move || {
        let result = receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 2_000),
            proceed,
        );
        (result, transfer.received_chunks())
    });
    let alice_address = between(&alice, &bob);
    let pause_after_four = |update: &TransferUpdate| {
        if update.next_index >= 4 {
            Gate::Pause
        } else {
            Gate::Proceed
        }
    };
    let first = push_chunks(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 2_001),
        pause_after_four,
    )
    .unwrap();

    assert_eq!(first.state, TransferState::Paused);
    assert_eq!(first.next_index, 4);
    assert_eq!(first.percent, 79, "four of six chunks is 79% of the bytes");
    assert!(transfer_part_exists(&root, offer.id), "part file survives");

    // Closing the session must not let the receiver call a partial file a success.
    drop(alice_stream);
    let (interrupted, stored) = receive.join().unwrap();
    assert!(interrupted.is_err(), "a truncated stream is not delivered");
    assert_eq!(stored, 4);

    // Session two: a fresh transfer restores the verified chunks from the sidecar index,
    // and the sender resumes exactly where that restore pointed.
    let mut resumed = begun(&root, offer.clone());
    assert_eq!(resumed.received_chunks(), 4, "verified chunks survive");
    assert_eq!(resumed.next_index(), 4, "the gap starts at chunk four");
    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let alice_address = between(&alice, &bob);
    let receive = thread::spawn(move || {
        let result = receive_chunks(
            &mut bob_stream,
            &mut resumed,
            &receiving(&bob_address, 3_000),
            proceed,
        );
        (result, resumed.state())
    });
    let second = push_chunks(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 3_001),
        proceed,
    )
    .unwrap();
    let (landed, engine_state) = receive.join().unwrap();
    let landed = landed.unwrap();

    assert_eq!(second.state, TransferState::Completed);
    assert_eq!(landed.state, TransferState::Completed);
    assert_eq!(engine_state, TransferState::Completed);
    assert_eq!(fs::read(landed.path.unwrap()).unwrap(), contents);
    assert_eq!(
        fs::read_dir(&root).unwrap().count(),
        2,
        "source + landed, no leftover index"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_receiver_pause_says_so_instead_of_calling_it_a_cancel() {
    let alice = Arc::new(Identity::from_secret([50; 32]));
    let bob = Arc::new(Identity::from_secret([51; 32]));
    let root = temp_dir("receiver-pause");
    let contents = one_chunk_payload();
    let source = write_source(&root, &contents);
    let offer = offer_for(&contents, "later.bin");
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive = thread::spawn(move || {
        let result = receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 7_000),
            |_update: &TransferUpdate| Gate::Pause,
        );
        (result, transfer.part_path().to_path_buf())
    });
    let alice_address = between(&alice, &bob);
    let sent = push_chunks(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 7_001),
        proceed,
    )
    .unwrap();
    let (paused, part_path) = receive.join().unwrap();
    let paused = paused.unwrap();

    assert_eq!(paused.state, TransferState::Paused);
    assert_eq!(paused.reason.as_deref(), Some(PAUSE_REASON));
    assert_eq!(sent.state, TransferState::Paused);
    assert_eq!(sent.reason, None, "a pause is not a failure");
    assert_eq!(sent.next_index, 0, "nothing was confirmed yet");
    assert!(
        part_path.exists(),
        "a pause keeps the part file for a resume"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_cancelled_receive_stops_the_sender_and_leaves_no_part_file() {
    let alice = Arc::new(Identity::from_secret([44; 32]));
    let bob = Arc::new(Identity::from_secret([45; 32]));
    let root = temp_dir("cancel");
    let contents = one_chunk_payload();
    let source = write_source(&root, &contents);
    let offer = offer_for(&contents, "gone.bin");
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive = thread::spawn(move || {
        receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 4_000),
            |_update: &TransferUpdate| Gate::Cancel,
        )
    });
    let alice_address = between(&alice, &bob);
    let sent = push_chunks(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 4_001),
        proceed,
    )
    .unwrap();
    let cancelled = receive.join().unwrap().unwrap();

    assert_eq!(cancelled.state, TransferState::Cancelled);
    assert_eq!(cancelled.reason.as_deref(), Some(CANCEL_REASON));
    assert_eq!(sent.state, TransferState::Cancelled);
    assert!(!transfer_part_exists(&root, offer.id));
    assert_eq!(
        fs::read_dir(&root).unwrap().count(),
        1,
        "only the sender's own copy remains"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tampered_bytes_fail_the_hash_and_are_never_kept() {
    let alice = Arc::new(Identity::from_secret([46; 32]));
    let bob = Arc::new(Identity::from_secret([47; 32]));
    let root = temp_dir("hash");
    let contents = six_chunk_payload();
    // The sender offers the hash of `contents` but streams a different file of the same
    // length: the receiver must reject it and delete every byte it stored.
    let mut tampered = contents.clone();
    tampered[MAX_FILE_CHUNK_BYTES * 5 + 500] ^= 0xff;
    let source = write_source(&root, &tampered);
    let offer = offer_for(&contents, "tampered.bin");
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive = thread::spawn(move || {
        let result = receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 5_000),
            proceed,
        );
        (result, transfer.received_chunks())
    });
    let alice_address = between(&alice, &bob);
    let sent = push_chunks(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 5_001),
        proceed,
    )
    .unwrap();
    let (received, stored) = receive.join().unwrap();

    let error = received.unwrap_err();
    assert!(error.contains("hash"), "{error}");
    assert_eq!(stored, 0, "a hash-bad file is thrown away");
    assert_eq!(sent.state, TransferState::Failed);
    assert!(sent.reason.unwrap().contains("hash"));
    assert!(!transfer_part_exists(&root, offer.id));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_chunk_from_another_transfer_is_rejected() {
    let alice = Arc::new(Identity::from_secret([48; 32]));
    let bob = Arc::new(Identity::from_secret([49; 32]));
    let root = temp_dir("foreign");
    let contents = one_chunk_payload();
    write_source(&root, &contents);
    let offer = offer_for(&contents, "mine.bin");
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive = thread::spawn(move || {
        let result = receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 6_000),
            proceed,
        );
        (result, transfer.received_chunks())
    });
    // Alice ignores what was actually asked for and streams a chunk belonging to another
    // transfer of hers.
    let accept: Envelope<Value> = alice_stream.receive().unwrap();
    assert_eq!(accept.kind, kind::FILE_ACCEPT);
    assert_eq!(accept.payload["transfer_id"], offer.id.to_string());
    let forged = frame(
        &between(&alice, &bob),
        kind::FILE_CHUNK,
        &FileChunkPayload {
            transfer_id: Uuid::now_v7(),
            index: 0,
            bytes: contents[..expected_chunk_len(offer.size, offer.chunk_count(), 0).unwrap()]
                .to_vec(),
            last: true,
        },
        Hlc::default(),
    )
    .unwrap();
    alice_stream.send(&forged).unwrap();
    let (result, stored) = receive.join().unwrap();

    assert!(result
        .unwrap_err()
        .contains("peer sent a file transfer frame that is not ours"));
    assert_eq!(stored, 0, "nothing suspect is stored");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_frame_addressed_to_someone_else_is_rejected() {
    let alice = Arc::new(Identity::from_secret([52; 32]));
    let bob = Arc::new(Identity::from_secret([53; 32]));
    let outsider = Identity::from_secret([54; 32]);
    let root = temp_dir("address");
    let contents = one_chunk_payload();
    write_source(&root, &contents);
    let offer = offer_for(&contents, "wrong-recipient.bin");
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let receive = thread::spawn(move || {
        receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 8_000),
            proceed,
        )
    });
    let _: Envelope<Value> = alice_stream.receive().unwrap();
    // Authenticated as Alice, but addressed to a peer who never asked for this file.
    let misplaced = TransferAddress {
        from: alice.peer_id(),
        to: outsider.peer_id(),
    };
    let chunk = frame(
        &misplaced,
        kind::FILE_CHUNK,
        &FileChunkPayload {
            transfer_id: offer.id,
            index: 0,
            bytes: contents.clone(),
            last: true,
        },
        Hlc::default(),
    )
    .unwrap();
    alice_stream.send(&chunk).unwrap();

    assert!(receive.join().unwrap().unwrap_err().contains("not ours"));
    fs::remove_dir_all(root).unwrap();
}

/// The pause and cancel reasons are part of the wire contract.
#[test]
fn pause_and_cancel_reasons_are_distinct() {
    assert_ne!(PAUSE_REASON, CANCEL_REASON);
    assert!(!PAUSE_REASON.is_empty());
    assert!(!CANCEL_REASON.is_empty());
}

/// A dispatcher that already read the `file_accept` envelope — which is what the desktop
/// session loop does before it starts a sender — can still stream the whole file.
#[test]
fn a_sender_given_the_accept_streams_the_whole_file() {
    let alice = Arc::new(Identity::from_secret([44; 32]));
    let bob = Arc::new(Identity::from_secret([45; 32]));
    let root = temp_dir("dispatched");
    let contents = one_chunk_payload();
    let source = write_source(&root, &contents);
    let offer = offer_for(&contents, "dispatched.bin");
    let mut transfer = begun(&root, offer.clone());

    let (mut alice_stream, mut bob_stream) = streams(Arc::clone(&alice), Arc::clone(&bob));
    let bob_address = between(&bob, &alice);
    let alice_address = between(&alice, &bob);
    let receive = thread::spawn(move || {
        receive_chunks(
            &mut bob_stream,
            &mut transfer,
            &receiving(&bob_address, 7_000),
            proceed,
        )
    });

    let envelope: Envelope<Value> = alice_stream.receive().unwrap();
    assert_eq!(envelope.kind, kind::FILE_ACCEPT);
    let frame_id = envelope.id;
    let accept: FileAcceptPayload = serde_json::from_value(envelope.payload).unwrap();
    assert_eq!(accept.transfer_id, offer.id);
    // Every frame the protocol sends gets an id of its own, so a transfer is named only
    // inside the payload. A dispatcher that looked for it in the envelope id would reject
    // every receiver that is not the app itself.
    assert_ne!(frame_id, accept.transfer_id);
    assert_eq!(accept.resume_from, 0, "a fresh receiver owes every chunk");

    let sent = push_accepted(
        &mut alice_stream,
        &sending(&alice_address, &offer, &source, 7_001),
        &accept,
        proceed,
    )
    .unwrap();
    let received = receive.join().unwrap().unwrap();

    assert_eq!(sent.state, TransferState::Completed);
    assert_eq!(received.state, TransferState::Completed);
    assert_eq!(
        fs::read(received.path.expect("the file landed")).unwrap(),
        contents
    );
    fs::remove_dir_all(root).unwrap();
}
