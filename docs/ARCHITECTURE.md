# Architecture

```text
packages/ui  ->  packages/bridge (PlatformBridge)  ->  apps/desktop (Tauri)
                                                ->  crates/alver-platform
                                                ->  crates/alver-core
```

`alver-core` has no Tauri or OS dependencies. `alver-platform` owns OS services. UI components use the bridge contract; `mockBridge` is an empty browser-development adapter and does not seed sample conversations.

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
- mDNS `_alverchat._tcp.local.` publication/browse and a signed, size-capped UDP broadcast beacon fallback.
- Tauri 2 main window, CSP and bridge commands. The native command surface currently returns no discovered peers; live transport has not been connected to it.

## Not connected yet

No TLS WebSocket transport, peer trust persistence, automatic outbox replay, or WebRTC session is active in this build. Native startup and browser development use empty peer/message lists. Chat delivery, discovery, calls, and trusted identity are unavailable until their transport integration is completed. Local message persistence is encrypted with a key from the OS credential store; cross-platform keychain behavior still needs release-platform validation.
