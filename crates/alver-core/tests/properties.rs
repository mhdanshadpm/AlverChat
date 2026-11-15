//! Property tests for the convergence rules in [`alver_core::rules`].
//!
//! These run against the pure layer, plus the encrypted store for the idempotent-apply
//! and tombstone-permanence families (kept to a small case count because every case
//! opens a fresh SQLCipher database).

use std::cmp::Ordering;

use proptest::prelude::*;
use uuid::Uuid;

use alver_core::files::expected_chunk_len;
use alver_core::protocol::MAX_FILE_CHUNK_BYTES;
use alver_core::rules::{self, EditWindow, ReceiptKind};
use alver_core::security;
use alver_core::{Hlc, Message, MessageLedger};

/// `logical` stays below `u32::MAX` so a tick or merge can never saturate, which would
/// legitimately break the "strictly newer" properties.
fn clock() -> impl Strategy<Value = Hlc> {
    (any::<u64>(), 0_u32..u32::MAX).prop_map(|(physical_ms, logical)| Hlc {
        physical_ms,
        logical,
    })
}

fn message(body: &str, sender: &str, at: u64) -> Message {
    Message {
        id: Uuid::now_v7(),
        chat_id: "chat-1".into(),
        sender: sender.into(),
        body: body.into(),
        clock: Hlc {
            physical_ms: at,
            logical: 0,
        },
        tombstone: false,
        forwarded: false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// HLC comparison is antisymmetric and reflexive: it is a partial order made total by
    /// `(physical_ms, logical)`, so two peers must never disagree about direction.
    #[test]
    fn hlc_compare_is_antisymmetric(a in clock(), b in clock()) {
        let forward = rules::compare_hlc(&a, &b);
        let backward = rules::compare_hlc(&b, &a);
        prop_assert_eq!(forward, backward.reverse());
        prop_assert_eq!(rules::compare_hlc(&a, &a), Ordering::Equal);
    }

    #[test]
    fn hlc_derived_order_is_total_and_transitive(a in clock(), b in clock(), c in clock()) {
        // Equal clocks are broken by the message id, so give each one a distinct id:
        // the comparator is then total and every permutation converges on one order.
        let items = [(a, Uuid::max()), (b, Uuid::nil()), (c, Uuid::from_u128(7))];
        let mut ascending = items;
        rules::sort_messages(&mut ascending, |item| *item);
        let mut rotated = [items[1], items[2], items[0]];
        rules::sort_messages(&mut rotated, |item| *item);
        prop_assert_eq!(ascending, rotated, "sorting must not depend on input order");

        let mut descending = items;
        descending.sort_by(|x, y| rules::compare_messages(&y.0, &y.1, &x.0, &x.1));
        let reversed: Vec<_> = ascending.iter().copied().rev().collect();
        prop_assert_eq!(descending.to_vec(), reversed);

        if rules::compare_hlc(&a, &b) == Ordering::Less
            && rules::compare_hlc(&b, &c) == Ordering::Less
        {
            prop_assert_eq!(rules::compare_hlc(&a, &c), Ordering::Less);
        }
        // The message comparator adds the id, so equal clocks still have one order.
        let (left, right) = (Uuid::max(), Uuid::nil());
        prop_assert_eq!(
            rules::compare_messages(&a, &left, &a, &right),
            Ordering::Greater
        );
        prop_assert_eq!(rules::compare_messages(&a, &left, &a, &left), Ordering::Equal);
    }

    /// A tick always produces a strictly newer clock, and merge always observes the
    /// remote: neither can move backwards even when the wall clock is skewed.
    #[test]
    fn tick_and_merge_strictly_advance(a in clock(), b in clock(), now in any::<u64>()) {
        prop_assert!(rules::compare_hlc(&a.tick(now), &a) == Ordering::Greater);
        let merged = a.merge(b, now);
        prop_assert!(rules::compare_hlc(&merged, &a) == Ordering::Greater);
        prop_assert!(rules::compare_hlc(&merged, &b) != Ordering::Less);
        prop_assert_eq!(
            merged.physical_ms,
            now.max(a.physical_ms).max(b.physical_ms)
        );
        // Merging is symmetric in the physical component.
        prop_assert_eq!(merged.physical_ms, b.merge(a, now).physical_ms);
    }

    #[test]
    fn duplicate_connection_tie_break_is_deterministic(
        a in "[a-z]{1,12}",
        b in "[a-z]{1,12}",
    ) {
        let winner = rules::tie_break_by_peer(&a, &b);
        let expected = if a <= b { a.clone() } else { b.clone() };
        prop_assert_eq!(winner.to_owned(), expected);
        if a == b {
            prop_assert_eq!(
                rules::duplicate_connection_winner(&a, &b),
                rules::ConnectionRole::Keep
            );
        } else {
            let local = rules::duplicate_connection_winner(&a, &b);
            let remote = rules::duplicate_connection_winner(&b, &a);
            prop_assert_ne!(local, remote, "exactly one side must drop");
        }
    }

    /// Idempotent apply: replaying a message never duplicates it and never re-fires the
    /// outbox, however many times the frame arrives.
    #[test]
    fn ledger_apply_is_idempotent(body in ".{0,40}", replays in 1_usize..8) {
        let mut ledger = MessageLedger::default();
        let entry = message(&body, "peer-a", 10);
        prop_assert!(ledger.apply(entry.clone()));
        for _ in 1..replays {
            prop_assert!(!ledger.apply(entry.clone()));
        }
        prop_assert_eq!(ledger.revisions(&entry.id).len(), 1);
        let mut tracker = rules::ApplyTracker::new();
        prop_assert!(tracker.apply(entry.id));
        prop_assert!(!tracker.apply(entry.id));
    }

    /// Tombstone permanence: once a message is deleted, no late edit, reaction or
    /// receipt may resurrect or decorate it.
    #[test]
    fn tombstones_are_permanent(body in ".{1,20}", at in 0_u64..1_000_000) {
        let mut ledger = MessageLedger::default();
        let entry = message(&body, "peer-a", at);
        ledger.apply(entry.clone());
        prop_assert!(ledger.tombstone(entry.id, "peer-a"));
        let resurrection = ledger.edit(
            entry.id,
            "peer-a",
            "resurrect",
            Hlc {
                physical_ms: at + 1,
                logical: 0,
            },
            at + 1,
        );
        prop_assert!(resurrection.is_err());
        let edited = ledger.get(&entry.id).unwrap();
        prop_assert!(edited.tombstone);
        prop_assert!(edited.body.is_empty());
        prop_assert!(!rules::late_edit_allowed(true));
        prop_assert!(!rules::late_reaction_allowed(true));
        prop_assert!(!rules::late_receipt_allowed(true));
        prop_assert!(rules::accepts_updates(false));
    }

    /// The edit window is inclusive at the boundary and refuses exactly one ms later.
    #[test]
    fn edit_window_boundary_is_inclusive(
        sent in 0_u64..u64::MAX / 4,
        window in 0_u64..u64::MAX / 4,
        skew in 0_u64..1_000,
    ) {
        let windowed = EditWindow::new(window);
        prop_assert!(windowed.is_open(sent, sent));
        prop_assert!(windowed.is_open(sent, sent.saturating_add(window)));
        prop_assert!(!windowed.is_open(sent, sent.saturating_add(window).saturating_add(1)));
        // A clock behind the message never expires it.
        prop_assert!(windowed.is_open(sent.saturating_add(skew + 1), sent));
        prop_assert!(EditWindow::new(u64::MAX).is_open(sent, u64::MAX));
    }

    /// Chunk bounds: an index is addressable exactly when it is below the chunk count,
    /// and the last chunk carries the remainder.
    #[test]
    fn chunk_indices_are_bounded(
        size in 0_u64..8_u64 * MAX_FILE_CHUNK_BYTES as u64,
        index in 0_u64..128,
    ) {
        let chunk_count = size.div_ceil(MAX_FILE_CHUNK_BYTES as u64);
        let expected = expected_chunk_len(size, chunk_count, index);
        prop_assert_eq!(expected.is_some(), index < chunk_count);
        if let Some(length) = expected {
            prop_assert!(length <= MAX_FILE_CHUNK_BYTES);
            let offset = index * MAX_FILE_CHUNK_BYTES as u64;
            prop_assert_eq!(length as u64, (size - offset).min(MAX_FILE_CHUNK_BYTES as u64));
            if index + 1 < chunk_count {
                prop_assert_eq!(length, MAX_FILE_CHUNK_BYTES);
            }
        }
        prop_assert_eq!(expected_chunk_len(size, chunk_count, u64::MAX), None);
    }

    /// Path traversal: a single-component safe name always validates.
    #[test]
    fn received_file_names_are_safe_components(
        stem in "[a-z][a-z0-9_-]{0,20}",
        extension in "[a-z]{1,4}",
    ) {
        let safe = format!("{stem}.{extension}");
        prop_assert!(security::is_safe_single_component(&safe), "{safe:?} must be safe");
        prop_assert!(security::validate_received_file_name(&safe).is_ok());
    }

    /// Every hostile spelling a peer could send is refused by the plain check.
    #[test]
    fn hostile_file_names_are_refused(hostile in prop::sample::select(vec![
        "",
        "../a.txt",
        "../../a.txt",
        "sub/../a.txt",
        "..",
        "/etc/passwd",
        "\\\\server\\share\\a.txt",
        "C:\\windows\\system32\\cmd.exe",
        "c:/windows/a.txt",
        "a\0b.txt",
        "a\nb.txt",
        "NUL.txt",
        "con",
        "COM1.a",
        "LPT9.txt",
        "trailing.",
        "trailing ",
    ])) {
        prop_assert!(
            security::validate_received_file_name(hostile).is_err(),
            "{hostile:?} must be refused"
        );
        prop_assert!(
            !security::is_safe_single_component(hostile),
            "{hostile:?} must never look deliverable"
        );
    }

    /// Length limits are enforced before anything touches the filesystem.
    #[test]
    fn oversized_file_names_are_refused(
        hostile in prop::sample::select(vec![
            "x".repeat(security::MAX_PATH_BYTES + 1),
            "y".repeat(security::MAX_FILE_NAME_BYTES + 1),
        ])
    ) {
        prop_assert!(security::validate_received_file_name(&hostile).is_err());
        prop_assert!(!security::is_safe_single_component(&hostile));
    }

    #[test]
    fn receipt_levels_never_downgrade(current in any::<bool>(), incoming in any::<bool>()) {
        let stored = if current { ReceiptKind::Read } else { ReceiptKind::Delivered };
        let arriving = if incoming { ReceiptKind::Read } else { ReceiptKind::Delivered };
        match arriving.cmp(&stored) {
            Ordering::Greater => prop_assert_eq!(
                rules::receipt_advances(Some(stored), arriving),
                Ok(true)
            ),
            Ordering::Equal => prop_assert_eq!(
                rules::receipt_advances(Some(stored), arriving),
                Ok(false)
            ),
            Ordering::Less => prop_assert_eq!(
                rules::receipt_advances(Some(stored), arriving),
                Err(rules::RuleViolation::StaleReceipt)
            ),
        }
        // With nothing stored yet, any level is an advance.
        prop_assert_eq!(rules::receipt_advances(None, arriving), Ok(true));
        prop_assert_eq!(ReceiptKind::parse(stored.as_str()), Some(stored));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// The encrypted store applies the same idempotence and permanence rules as the
    /// in-memory ledger, and keeps its FTS index in step.
    #[test]
    fn store_apply_is_idempotent_and_tombstones_win(body in "[a-z][a-z ]{0,23}", replays in 1_usize..5) {
        let mut store = alver_core::storage::Store::open(":memory:", &[3; 32])
            .map_err(proptest::test_runner::TestCaseError::fail)?;
        let entry = message(&body, "peer-a", 10);
        let first = store
            .save_message(&entry)
            .map_err(proptest::test_runner::TestCaseError::fail)?;
        prop_assert!(first);
        for _ in 1..replays {
            let again = store
                .save_message(&entry)
                .map_err(proptest::test_runner::TestCaseError::fail)?;
            prop_assert!(!again);
        }
        prop_assert_eq!(
            store
                .messages_for_chat("chat-1")
                .map_err(proptest::test_runner::TestCaseError::fail)?
                .len(),
            1
        );
        prop_assert!(store
            .search_messages(&body, 10)
            .map_err(proptest::test_runner::TestCaseError::fail)?
            .iter()
            .any(|(id, _)| *id == entry.id));
        prop_assert!(store
            .add_reaction(entry.id, "peer-b", "heart", 11)
            .map_err(proptest::test_runner::TestCaseError::fail)?);
        prop_assert!(!store
            .add_reaction(entry.id, "peer-b", "heart", 12)
            .map_err(proptest::test_runner::TestCaseError::fail)?);
        prop_assert!(store
            .tombstone_message(entry.id, "peer-a")
            .map_err(proptest::test_runner::TestCaseError::fail)?);
        // Gone from search, invisible reactions, immune to late edits.
        prop_assert!(store
            .search_messages(&body, 10)
            .map_err(proptest::test_runner::TestCaseError::fail)?
            .is_empty());
        prop_assert!(store
            .reactions_for_message(entry.id)
            .map_err(proptest::test_runner::TestCaseError::fail)?
            .is_empty());
        prop_assert!(!store
            .add_reaction(entry.id, "peer-c", "boo", 13)
            .map_err(proptest::test_runner::TestCaseError::fail)?);
        prop_assert!(!store
            .record_receipt(entry.id, "peer-b", ReceiptKind::Read, 14)
            .map_err(proptest::test_runner::TestCaseError::fail)?);
    }
}
