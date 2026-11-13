# AlverChat

Serverless local network chat for teams. The desktop shell uses Tauri 2; the UI communicates through `PlatformBridge`, with a mock implementation for offline development.

## Development

```sh
pnpm install
pnpm dev
```

Desktop development: `pnpm tauri dev`. Production UI bundle: `pnpm build`. Rust workspace: `cargo build --workspace`.

The current build is a UI and architecture foundation. Messaging, discovery, identity storage, calls, and native overlay integrations are being added phase by phase; the mock data is local to the app and does not contact a server.

## Platform notes

Dynamic peer WebSocket ports, mDNS UDP 5353, and the configurable discovery UDP port must be allowed on private/LAN networks. WebRTC media requires a UDP range configured by the platform layer. macOS signing/notarization and Windows installer setup require platform certificates/runtime packaging configuration.

## Adding a platform

Implement `PlatformServices` for windowing, tray/menu bar, hotkeys, notifications, startup, permissions, firewall help, and deep links. Implement `PlatformBridge` for the target runtime and retain `alver-core` without OS or Tauri dependencies. Mobile clients must account for iOS background network suspension.
