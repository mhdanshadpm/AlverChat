# AlverChat

## Releases and automatic updates

Desktop release builds are created for Windows x64 and Apple Silicon macOS when a `v*` tag is pushed. The updater verifies every downloaded bundle against the public key in `apps/desktop/src-tauri/tauri.conf.json` before installing it. An update is only offered for a platform whose signed artifact is in the release — `…_x64-setup.exe.sig` for Windows and `…_aarch64.app.tar.gz.sig` for macOS — which is why `bundle.targets` lists `nsis`, `app` and `dmg`: the `app` target, not the `dmg`, is what produces the macOS updater archive.

To enable signed releases in a new GitHub repository:

1. Add the contents of your private Tauri updater key as the `TAURI_SIGNING_PRIVATE_KEY` repository secret. Keep the private key outside the repository and back it up securely; existing installations depend on the matching key.
2. Optionally set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the private key has a password.
3. Push a version tag such as `v0.1.2`. GitHub Actions builds the installer and publishes the signed update manifest and artifacts.

The updater endpoint is set from the repository name by the release workflow. Replacing or losing the signing key breaks updates for previously installed builds.

AlverChat is a local network desktop chat project built with Tauri 2. The desktop app provides signed LAN peer discovery, authenticated encrypted direct-message transport over TLS 1.3 WebSocket with Noise XX, encrypted local message storage, persistent identity, draggable overlay windows, shared bubble preferences, and signed GitHub release updates.

## Development

```sh
pnpm install
pnpm dev
```

Desktop development: `pnpm dev:desktop`. Production UI bundle: `pnpm build`. Rust workspace: `cargo build --workspace`.

Several installs can share one machine with `pnpm dev:peers [count]`, which builds the desktop binary, starts the dev server and launches that many instances, each with its own profile under `.dev-peers/peer-N`. It is the same three variables below — `ALVERCHAT_MOCK_PEER` names the instance so two copies never merge their identity, `ALVERCHAT_DATA_DIR` and `ALVERCHAT_CONFIG_DIR` point it at its own folders — and it is how the cross-device checks were run on one Mac.

Secrets — the database key, this device's identity and the LAN passphrase — live in `secrets/` inside the app data directory as `0600` files, so the app never asks macOS for your login password and a rebuilt dev binary keeps working. Point `ALVERCHAT_DATA_DIR` (and `ALVERCHAT_CONFIG_DIR`) at a private folder to give a test instance its own profile; `ALVERCHAT_MOCK_PEER` gives it a name of its own as well, which is what separates one instance's identity from another's. `ALVERCHAT_KEYCHAIN=1` switches back to the OS credential store; a profile that was created there is migrated into `secrets/` on the first run, which is the last time the keychain is asked.

Browser mode is only a UI development shell; it has no peer service and rejects message sends. The desktop UI does not seed sample conversations or claim that unsupported features succeeded. See [docs/PROGRESS.md](docs/PROGRESS.md) and [docs/BLOCKERS.md](docs/BLOCKERS.md) for current capability and release status.

## Platform notes

AlverChat listens for peers on a dynamic TCP port and advertises that port over mDNS (UDP 5353, multicast); Settings → Privacy & Network shows the live listener and discovery port (UDP 47986). The same signed beacon is also broadcast on UDP 47986 every three seconds, which is how a peer is still found on a network that drops multicast — so both the multicast and that port matter to the firewall, not just one. WebRTC call media uses host candidates on the LAN, so the webview opens ephemeral UDP ports for audio, video and screen share. Allow the app itself — not a single fixed port — on private/LAN networks: macOS asks for Local Network access on first launch, and Windows offers the same under Allow an app through firewall. macOS signing/notarization and Windows installer setup require platform certificates/runtime packaging configuration.

## Adding a platform

Implement `PlatformServices` for windowing, tray/menu bar, hotkeys, notifications, startup, permissions, firewall help, and deep links. Implement `PlatformBridge` for the target runtime and retain `alver-core` without OS or Tauri dependencies. Mobile clients must account for iOS background network suspension.
