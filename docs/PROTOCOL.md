# Protocol v1

Every control frame uses an envelope:

```json
{"v":1,"id":"<uuidv7>","type":"message","from":"<peer-id>","to":"<chat-id>","hlc":{"physical_ms":0,"logical":0},"payload":{}}
```

Unknown envelope types are ignored. Control JSON is capped at 1 MiB; file chunks are capped at 64 KiB. Signed discovery beacons bind the peer ID and endpoint and are capped at 1 KiB. The core validates protocol versions and frame sizes, but the TLS WebSocket listener/client and binary file chunk exchange remain to be implemented.
