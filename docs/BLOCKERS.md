# Blockers

- 2026-11-13: TLS WebSocket integration was deferred after three build iterations because the selected tungstenite configuration API was non-exhaustive and the transport module was not compiling. The incomplete module and dependency changes were removed to keep the branch green. Retry with the crate's builder/config mutation API and add a certificate-pin verifier before wiring transport into Tauri.
