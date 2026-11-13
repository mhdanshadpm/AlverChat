# Architecture

```text
packages/ui  ->  packages/bridge (PlatformBridge)  ->  apps/desktop (Tauri)
                                                ->  crates/alver-platform
                                                ->  crates/alver-core
```

`alver-core` has no Tauri or OS dependencies. `alver-platform` owns OS services. UI components use the bridge contract; `mockBridge` keeps gallery and demo states available with no LAN.

## Implemented foundation

- Versioned protocol envelope, hybrid logical clock, peer-key fingerprint, idempotent in-memory ledger, sender-only tombstones.
- Ed25519 install identity primitive, Argon2id LAN key derivation, XChaCha20-Poly1305 payload helper.
- SQLCipher-backed SQLite open/migration with peers, chats, messages, FTS and outbox tables.
- Tauri 2 main window and CSP. The native command surface is intentionally minimal while the bridge and domain commands are being wired.

## Not connected yet

No mDNS/browser transport, UDP fallback, peer trust persistence, OS keychain storage, durable outbox replay, OS overlays, or WebRTC session is active in this build. The visible `8 peers` and chat records are explicitly mock content. Do not use the current mock identity/database as a production security boundary.
