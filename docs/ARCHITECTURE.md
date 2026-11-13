# Architecture

```text
packages/ui  ->  packages/bridge (PlatformBridge)  ->  apps/desktop (Tauri)
                                                ->  crates/alver-platform
                                                ->  crates/alver-core
```

`alver-core` has no Tauri or OS dependencies. `alver-platform` owns OS services. UI components use the bridge contract; `emptyBrowserBridge` is a browser-development adapter without peer data, and its message-send operation fails explicitly.

## Implemented foundation

- Versioned protocol envelope, hybrid logical clock, peer-key fingerprint/signatures, idempotent in-memory ledger, sender-only tombstones.
- Ed25519 install identity primitive, Argon2id LAN key derivation, XChaCha20-Poly1305 payload helper.
- Admin-signed group roster state with canonical member ordering, epoch checks, immutable Everyone-group roster, and atomic persistence in the SQLCipher store.
- Sender-only message edits with revision history, sender-only permanent tombstones, and FTS removal when edited/deleted.
- Local FTS search quotes user terms and caps query length, term count, and result count; edits replace indexed text and tombstones remove it.
- File receive primitive validates peer-supplied names and size, bounds chunk size/offsets, writes under an internal transfer ID, and checks BLAKE3 before finalizing.
- Encrypted backup container primitive uses a fresh Argon2id salt and XChaCha20-Poly1305 nonce; it accepts opaque bytes and is not yet wired to database/avatar export.
- SQLCipher-backed SQLite open/migration with peers, chats, messages, FTS and an idempotent persistent outbox API.
- Desktop opens the SQLCipher message store under the app data directory and obtains its database key through the platform credential store; message history is read in chat-clock order and survives restart.
- The desktop app loads a stable Ed25519 identity from the OS credential store, publishes signed mDNS announcements, rejects malformed or invalid signatures, and emits verified discovered peers to the UI.
- Tauri 2 main window, CSP, native dock/compact/menu windows, drag/position commands, overlay hit testing, and an update check/install/relaunch flow backed by signed GitHub Release artifacts.

## Network messaging status

The desktop messaging slice listens on a bounded TCP port, authenticates signed Ed25519 identities via TLS 1.3 `tokio-tungstenite` and Noise XX handshake, pins peer keys, encrypts message frames, stores outgoing messages in the SQLCipher outbox before delivery, retries offline messages, and stores received messages before acknowledging them. Signed mDNS announcements provide peer endpoints and identity keys.

Group membership and fan-out, message edits/reactions/read receipts over the network, profile/avatar sync, calls/WebRTC, heartbeat-based presence, UDP discovery fallback, and backup/export/restore are not connected to desktop commands. Some UI actions remain disabled or unavailable. File transfer is connected: the receiver pulls `file_chunk` frames over an authenticated session and verifies the whole file by blake3 (see `docs/PROTOCOL.md`). Browser development uses an empty adapter and never fakes message delivery. Cross-platform keychain, native overlay behavior, code signing, and notarization still need release-platform validation.
