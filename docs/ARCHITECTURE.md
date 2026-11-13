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

## Not connected yet

There is no TLS WebSocket transport, peer trust store, heartbeat, online presence, automatic outbox replay, or WebRTC session. mDNS discovery is live, but peers are shown as discovered and unavailable; `send_message` intentionally returns an unavailable error rather than persisting a false success. Browser development uses an empty mock adapter. Group membership, calls, files, profile/avatar sync, and backup/export/restore are not wired to network commands. Local message persistence is encrypted with a key from the OS credential store; cross-platform keychain behavior still needs release-platform validation.
