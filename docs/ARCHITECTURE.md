# Architecture

```text
packages/ui  ->  packages/bridge (PlatformBridge)  ->  apps/desktop (Tauri)
                                                ->  crates/alver-platform
                                                ->  crates/alver-core
```

`alver-core` has no Tauri or OS dependencies. `alver-platform` owns OS services. UI components use the bridge contract; `mockBridge` keeps gallery and demo states available with no LAN.

## Implemented foundation

- Versioned protocol envelope, hybrid logical clock, peer-key fingerprint/signatures, idempotent in-memory ledger, sender-only tombstones.
- Ed25519 install identity primitive, Argon2id LAN key derivation, XChaCha20-Poly1305 payload helper.
- Admin-signed group roster state with canonical member ordering, epoch checks, immutable Everyone-group roster, and atomic persistence in the SQLCipher store.
- Sender-only message edits with revision history, sender-only permanent tombstones, and FTS removal when edited/deleted.
- File receive primitive validates peer-supplied names and size, bounds chunk size/offsets, writes under an internal transfer ID, and checks BLAKE3 before finalizing.
- SQLCipher-backed SQLite open/migration with peers, chats, messages, FTS and an idempotent persistent outbox API.
- mDNS `_alverchat._tcp.local.` publication/browse and a signed, size-capped UDP broadcast beacon fallback.
- Tauri 2 main window, CSP and bridge commands. The native command surface currently returns no discovered peers; live transport has not been connected to it.

## Not connected yet

No TLS WebSocket transport, peer trust persistence, OS keychain storage, automatic outbox replay, OS overlays, or WebRTC session is active in this build. The visible `8 peers` and chat records are explicitly mock content. Do not use the current mock identity/database as a production security boundary.
