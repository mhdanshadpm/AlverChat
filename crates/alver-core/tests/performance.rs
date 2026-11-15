//! Opt-in throughput evidence, not a CI gate.
//!
//! Run with:
//! `cargo test -p alver-core --release --test performance -- --ignored --nocapture`
//!
//! Numbers are informational: they answer "is the protocol path fast enough to
//! stay invisible next to TLS and SQLite?" on the machine that ran them, and are
//! recorded in `docs/DECISIONS.md` with the date they were measured.

use std::time::Instant;

use alver_core::files::hash_bytes;
use alver_core::protocol::{decode_control_frame, MAX_FILE_CHUNK_BYTES};
use alver_core::{Envelope, Hlc, Message, MessageLedger, PROTOCOL_VERSION};
use uuid::Uuid;

fn measure(label: &str, iterations: u32, mut body: impl FnMut(u32)) {
    let started = Instant::now();
    for index in 0..iterations {
        body(index);
    }
    let elapsed = started.elapsed();
    let per_op = elapsed.as_secs_f64() / f64::from(iterations);
    println!(
        "{label}: {iterations} iterations in {:.3}s → {:.2} µs/op ({:.0} ops/s)",
        elapsed.as_secs_f64(),
        per_op * 1_000_000.0,
        1.0 / per_op
    );
}

fn message_frame() -> Vec<u8> {
    serde_json::to_vec(&Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: "message".into(),
        from: "a".repeat(64),
        to: "a".repeat(64),
        hlc: Hlc {
            physical_ms: 1_760_000_000_000,
            logical: 3,
        },
        payload: serde_json::json!({ "text": "x".repeat(256) }),
    })
    .unwrap()
}

#[test]
#[ignore = "opt-in throughput evidence; run with --ignored --nocapture"]
fn protocol_path_throughput() {
    let frame = message_frame();
    println!("envelope size: {} bytes", frame.len());

    measure("decode_control_frame", 50_000, |_| {
        assert!(decode_control_frame(&frame).unwrap().is_some());
    });

    let envelope: Envelope<serde_json::Value> = serde_json::from_slice(&frame).unwrap();
    measure("encode envelope", 50_000, |_| {
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(!bytes.is_empty());
    });

    let mut clock = Hlc::default();
    measure("hlc tick + merge", 1_000_000, |index| {
        clock = clock.tick(1_760_000_000_000 + u64::from(index) / 1000);
        clock = clock.merge(
            Hlc {
                physical_ms: 1_760_000_000_000,
                logical: index,
            },
            1_760_000_000_000,
        );
    });

    let chunk = vec![7_u8; MAX_FILE_CHUNK_BYTES];
    let started = Instant::now();
    let churn = 2_000;
    for _ in 0..churn {
        let _ = hash_bytes(&chunk);
    }
    let megabytes = (chunk.len() as f64 * f64::from(churn)) / (1024.0 * 1024.0);
    println!(
        "blake3 40 KiB chunk: {churn} hashes in {:.3}s → {:.0} MiB/s",
        started.elapsed().as_secs_f64(),
        megabytes / started.elapsed().as_secs_f64()
    );

    let mut ledger = MessageLedger::default();
    let sender = "a".repeat(64);
    measure("MessageLedger::apply", 100_000, |index| {
        let applied = ledger.apply(Message {
            id: Uuid::from_u128(u128::from(index) + 1),
            chat_id: sender.clone(),
            sender: sender.clone(),
            body: "performance evidence".into(),
            clock: Hlc::default(),
            tombstone: false,
            forwarded: false,
        });
        assert!(applied);
    });
}
