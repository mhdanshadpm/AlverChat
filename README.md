# AlverChat

## Releases and automatic updates

Desktop release builds are created for Windows x64 and Apple Silicon macOS when a `v*` tag is pushed. The updater verifies every downloaded bundle against the public key in `apps/desktop/src-tauri/tauri.conf.json` before installing it.

To enable signed releases in a new GitHub repository:

1. Add the contents of your private Tauri updater key as the `TAURI_SIGNING_PRIVATE_KEY` repository secret. Keep the private key outside the repository and back it up securely; existing installations depend on the matching key.
2. Optionally set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the private key has a password.
3. Push a version tag such as `v0.1.1`. GitHub Actions builds the installer and publishes the signed update manifest and artifacts.

The updater endpoint is set from the repository name by the release workflow. Replacing or losing the signing key breaks updates for previously installed builds.

AlverChat is a local network desktop chat project built with Tauri 2. The desktop app currently provides signed LAN peer discovery, an interim authenticated encrypted direct-message transport, encrypted local message storage, persistent identity, draggable overlay windows, shared bubble preferences, and signed GitHub release updates. The transport does not yet meet the specified TLS 1.3 WebSocket design, so production releases remain blocked.

## Development

```sh
pnpm install
pnpm dev
```

Desktop development: `pnpm dev:desktop`. Production UI bundle: `pnpm build`. Rust workspace: `cargo build --workspace`.

Browser mode is only a UI development shell; it has no peer service and rejects message sends. The desktop UI does not seed sample conversations or claim that unsupported features succeeded. See [docs/PROGRESS.md](docs/PROGRESS.md) and [docs/BLOCKERS.md](docs/BLOCKERS.md) for current capability and release status.

## Platform notes

Dynamic peer WebSocket ports, mDNS UDP 5353, and the configurable discovery UDP port must be allowed on private/LAN networks. WebRTC media requires a UDP range configured by the platform layer. macOS signing/notarization and Windows installer setup require platform certificates/runtime packaging configuration.

## Adding a platform

Implement `PlatformServices` for windowing, tray/menu bar, hotkeys, notifications, startup, permissions, firewall help, and deep links. Implement `PlatformBridge` for the target runtime and retain `alver-core` without OS or Tauri dependencies. Mobile clients must account for iOS background network suspension.
