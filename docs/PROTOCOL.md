# Protocol v1

Every control frame uses an envelope:

```json
{"v":1,"id":"<uuidv7>","type":"message","from":"<peer-id>","to":"<chat-id>","hlc":{"physical_ms":0,"logical":0},"payload":{}}
```

Unknown envelope types are ignored. Control JSON is capped at 1 MiB; file chunks are capped at 64 KiB. Signed mDNS discovery beacons bind the peer ID and advertised endpoint. The desktop currently carries message envelopes over an authenticated Noise XX/TCP session with pinned Ed25519 identities and bounded encrypted frames. This is interim: the fixed brief requires TLS 1.3 over `tokio-tungstenite`, which remains a release blocker. Binary file chunk exchange is also not connected.

Group rosters are represented by `GroupState`: the admin signs a canonical roster, member updates increment the epoch by one, and receivers can use `verify_successor_of` against the last accepted state to reject stale or skipped updates. A roster must contain its admin. The built-in `everyone` group rejects roster mutation. Group creation is disabled in the UI until membership storage and transport are connected.
