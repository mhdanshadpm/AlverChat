//! Schema-history tests: a populated v1 database must upgrade in place and keep every
//! row, and the runner must stay ordered, additive and non-destructive.

use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

use alver_core::groups::GroupState;
use alver_core::identity::Identity;
use alver_core::rules::ReceiptKind;
use alver_core::storage::{Store, LATEST_SCHEMA_VERSION, MIGRATIONS};
use alver_core::store_ext::{self, setting};
use alver_core::{Envelope, Hlc, Message, PROTOCOL_VERSION};

fn key() -> [u8; 32] {
    [42; 32]
}

fn database_path(label: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("alverchat-migrate-{label}-{}.db", Uuid::now_v7()));
    fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(path.file_name().unwrap())
}

fn message(id: Uuid, chat_id: &str, sender: &str, body: &str, at: u64) -> Message {
    Message {
        id,
        chat_id: chat_id.into(),
        sender: sender.into(),
        body: body.into(),
        clock: Hlc {
            physical_ms: at,
            logical: 0,
        },
        tombstone: false,
    }
}

/// Everything the v1 schema could hold, written before any v2+ table exists.
struct Seed {
    first: Uuid,
    second: Uuid,
    group_id: String,
    admin: Identity,
}

fn seed_v1_rows(store: &mut Store) -> Seed {
    let admin = Identity::from_secret([7; 32]);
    let member = Identity::from_secret([8; 32]);
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    store
        .save_message(&message(
            first,
            "chat-1",
            "peer-a",
            "the first message",
            100,
        ))
        .unwrap();
    store
        .save_message(&message(
            second,
            "chat-1",
            "peer-b",
            "the second message",
            200,
        ))
        .unwrap();
    // v1 revisions, peers, groups and outbox all exist from the start.
    store
        .edit_message(
            first,
            "peer-a",
            "the edited message",
            Hlc {
                physical_ms: 300,
                logical: 0,
            },
        )
        .unwrap();
    store
        .save_peer(
            &member.peer_id(),
            "Robin",
            &member.verifying_key().to_bytes(),
            50,
        )
        .unwrap();
    let group = GroupState::create(
        &admin,
        "group-seeded".into(),
        "Seeded".into(),
        [member.peer_id()],
    )
    .unwrap();
    let envelope = Envelope {
        v: PROTOCOL_VERSION,
        id: second,
        kind: "message".into(),
        from: "local".into(),
        to: "chat-1".into(),
        hlc: Hlc {
            physical_ms: 200,
            logical: 0,
        },
        payload: serde_json::json!({ "body": "the second message" }),
    };
    store
        .enqueue_message(
            &message(second, "chat-1", "peer-b", "the second message", 200),
            "chat-1",
            &envelope,
        )
        .ok();
    store.save_group_state(&group).unwrap();
    Seed {
        first,
        second,
        group_id: group.id.clone(),
        admin,
    }
}

#[test]
fn migrations_are_ordered_and_never_drop_anything() {
    assert_eq!(MIGRATIONS.first().map(|entry| entry.version), Some(1));
    assert!(MIGRATIONS
        .windows(2)
        .all(|pair| pair[1].version == pair[0].version + 1));
    assert_eq!(
        LATEST_SCHEMA_VERSION,
        MIGRATIONS.last().map(|entry| entry.version).unwrap()
    );
    for entry in MIGRATIONS {
        let lowered = entry.sql.to_lowercase();
        assert!(
            !lowered.contains("drop "),
            "migration {} must be additive",
            entry.version
        );
        assert!(
            !lowered.contains("user_version"),
            "migration {} must not bump its own version",
            entry.version
        );
        // Only v1 may create the message tables the desktop already depends on.
        if entry.version > 1 {
            assert!(
                !lowered.contains("create table if not exists messages"),
                "migration {} must not recreate v1 tables",
                entry.version
            );
        }
    }
    assert!(MIGRATIONS.len() >= 4);
}

#[test]
fn a_populated_v1_database_upgrades_with_every_row_intact() {
    let path = database_path("upgrade");
    let seed = {
        let mut store = Store::open_to_version(path.to_str().unwrap(), &key(), 1).unwrap();
        assert_eq!(store.schema_version().unwrap(), 1);
        let seed = seed_v1_rows(&mut store);
        // v1 has no reactions table at all.
        store
            .add_reaction(seed.first, "peer-c", "heart", 400)
            .unwrap_err();
        seed
    };

    let mut store = Store::open(path.to_str().unwrap(), &key()).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_SCHEMA_VERSION);

    // Every v1 row survived the upgrade.
    // The edit at line 300 ms moved `first` after `second` in HLC order, which is
    // exactly how the live chat list sorts, so the rows are checked positionally.
    let history = store.messages_for_chat("chat-1").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, seed.second);
    assert_eq!(history[0].body, "the second message");
    assert_eq!(history[1].id, seed.first);
    assert_eq!(history[1].body, "the edited message");
    assert_eq!(
        store.get_message(seed.second).unwrap().unwrap().sender,
        "peer-b"
    );
    assert_eq!(store.revision_count(seed.first).unwrap(), 1);
    assert_eq!(store.known_peers().unwrap().len(), 1);
    let group = store.get_group_state(&seed.group_id).unwrap().unwrap();
    assert_eq!(group.epoch, 1);
    assert!(group.verify());
    assert!(store.is_message_pending(seed.second).unwrap());

    // ...and the v2+ tables are now usable.
    assert!(store
        .add_reaction(seed.first, "peer-c", "heart", 400)
        .unwrap());
    assert!(store
        .record_receipt(seed.first, "peer-c", ReceiptKind::Read, 410)
        .unwrap());
    assert!(store.pin(seed.first, 0, 420).unwrap());
    assert!(store.star(seed.second, 430).unwrap());
    store
        .set_setting(setting::EDIT_WINDOW_MS, &900_u64, 440)
        .unwrap();
    assert_eq!(
        store.edit_window().unwrap(),
        alver_core::rules::EditWindow::new(900)
    );
    let announcement = store_ext::Announcement {
        id: Uuid::now_v7(),
        group_id: seed.group_id.clone(),
        sender: seed.admin.peer_id(),
        body: "standup moved".into(),
        clock: Hlc {
            physical_ms: 450,
            logical: 0,
        },
        admins_only: true,
        created_ms: 450,
    };
    store.save_announcement(&announcement).unwrap();
    let summary = store
        .acknowledgements_for_announcement(announcement.id)
        .unwrap()
        .unwrap();
    assert_eq!(summary.expected, 2);
    assert_eq!(summary.acked, 0);
    assert!(store
        .acknowledge_announcement(announcement.id, &seed.admin.peer_id(), 460)
        .unwrap());
    assert_eq!(
        store
            .acknowledgements_for_announcement(announcement.id)
            .unwrap()
            .unwrap()
            .acked,
        1
    );
    store
        .save_call_event(&store_ext::CallEvent {
            id: Uuid::now_v7(),
            chat_id: "chat-1".into(),
            call_id: Uuid::now_v7(),
            peer_id: "peer-a".into(),
            kind: store_ext::CallEventKind::Offer,
            accepted: None,
            started_ms: 470,
            ended_ms: None,
            note: String::new(),
        })
        .unwrap();
    assert_eq!(store.calls_for_chat("chat-1").unwrap().len(), 1);

    // The v1 columns all survive; v5 only appends the reply pointer.
    let columns = store.table_columns("messages").unwrap();
    assert_eq!(
        columns,
        vec![
            "id",
            "chat_id",
            "sender",
            "body",
            "physical_ms",
            "logical",
            "tombstone",
            "reply_to"
        ]
    );
    assert!(store.reply_to_for(seed.second).unwrap().is_none());
    drop(store);
    fs::remove_file(&path).ok();
}

#[test]
fn a_v4_database_gains_the_reply_column_and_keeps_its_rows() {
    let path = database_path("reply-column");
    let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
    {
        let mut store = Store::open_to_version(path.to_str().unwrap(), &key(), 4).unwrap();
        assert_eq!(store.schema_version().unwrap(), 4);
        store
            .save_message(&message(first, "chat-5", "peer-a", "quote me", 100))
            .unwrap();
        store
            .save_message(&message(second, "chat-5", "peer-b", "quoted you", 200))
            .unwrap();
        // The column does not exist yet, so a quote cannot be recorded before v5.
        store.set_reply_to(second, Some(first)).unwrap_err();
    }
    let mut store = Store::open(path.to_str().unwrap(), &key()).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
    assert_eq!(store.messages_for_chat("chat-5").unwrap().len(), 2);
    assert_eq!(
        store.messages_for_chat("chat-5").unwrap()[0].body,
        "quote me"
    );
    assert!(store.set_reply_to(second, Some(first)).unwrap());
    assert_eq!(store.reply_to_for(second).unwrap(), Some(first));
    assert_eq!(
        store.replies_for_chat("chat-5").unwrap(),
        vec![(second, first)]
    );
    // Quoting across chats and quoting yourself are both refused.
    store
        .set_reply_to(second, Some(Uuid::now_v7()))
        .unwrap_err();
    store.set_reply_to(second, Some(second)).unwrap_err();
    assert_eq!(store.reply_to_for(second).unwrap(), Some(first));
    assert!(store.set_reply_to(second, None).unwrap());
    assert!(store.reply_to_for(second).unwrap().is_none());
    assert!(store.replies_for_chat("chat-5").unwrap().is_empty());
    assert!(!store.set_reply_to(Uuid::now_v7(), Some(first)).unwrap());
    drop(store);
    fs::remove_file(&path).ok();
}

#[test]
fn the_runner_never_reapplies_or_downgrades() {
    let path = database_path("idempotent");
    {
        let mut store = Store::open(path.to_str().unwrap(), &key()).unwrap();
        store
            .save_message(&message(
                Uuid::now_v7(),
                "chat-loop",
                "peer-a",
                "survives a reopen",
                10,
            ))
            .unwrap();
        assert_eq!(store.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
    }
    // Reopening is a no-op, not a second application of the whole history.
    let reopened = Store::open(path.to_str().unwrap(), &key()).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
    assert_eq!(reopened.messages_for_chat("chat-loop").unwrap().len(), 1);
    drop(reopened);

    // Asking for an older schema is refused instead of dropping tables.
    let error = Store::open_to_version(path.to_str().unwrap(), &key(), 1)
        .err()
        .expect("a downgrade request must be refused instead of dropping tables");
    assert!(
        error.contains("downgrade") || error.contains("version"),
        "unexpected error: {error}"
    );
    let still_there = Store::open(path.to_str().unwrap(), &key()).unwrap();
    assert_eq!(still_there.schema_version().unwrap(), LATEST_SCHEMA_VERSION);
    assert_eq!(still_there.messages_for_chat("chat-loop").unwrap().len(), 1);
    assert!(Store::open_to_version(path.to_str().unwrap(), &key(), 99).is_err());
    drop(still_there);
    fs::remove_file(&path).ok();
}

#[test]
fn a_half_applied_migration_is_not_recorded() {
    let path = database_path("rollback");
    {
        let store = Store::open_to_version(path.to_str().unwrap(), &key(), 2).unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);
    }
    // Jumping straight to the latest version applies only what is missing.
    let store = Store::open_to_version(path.to_str().unwrap(), &key(), 4).unwrap();
    assert_eq!(store.schema_version().unwrap(), 4);
    drop(store);
    fs::remove_file(&path).ok();
}
