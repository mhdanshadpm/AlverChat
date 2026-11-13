# Protocol v1

Every control frame uses an envelope:

```json
{"v":1,"id":"<uuidv7>","type":"message","from":"<peer-id>","to":"<chat-id>","hlc":{"physical_ms":0,"logical":0},"payload":{}}
```

Unknown envelope types are ignored. Control JSON is capped at 1 MiB. Signed mDNS discovery beacons bind the peer ID and advertised endpoint. The desktop carries message envelopes over an authenticated TLS 1.3 `tokio-tungstenite` session and Noise XX session with pinned Ed25519 identities and bounded encrypted frames.

Group rosters are represented by `GroupState`: the admin signs a canonical roster, member updates increment the epoch by one, and receivers can use `verify_successor_of` against the last accepted state to reject stale or skipped updates. A roster must contain its admin. The built-in `everyone` group rejects roster mutation. Group creation is disabled in the UI until membership transport is connected.

## File transfer

The sender stages the bytes and enqueues one `file_offer{transfer_id, chat_id, file_name, size, blake3_hash, note}`; it carries no file data. The receiver answers on a fresh authenticated session with `file_accept{transfer_id, resume_from}`, and the sender streams `file_chunk{transfer_id, index, bytes}` in a window of four, each batch answered by `file_ack{transfer_id, contiguous_index, complete}`. `file_cancel{transfer_id, reason}` ends a paused (`paused`), cancelled (`cancelled`) or refused transfer.

- Every frame names its transfer in `payload.transfer_id`. The envelope `id` is the frame's own id, except for an offer, which the desktop builds with the transfer id so outbox retries keep their receipt identity.
- Chunks are base64 in the JSON payload and capped at 40 KiB decoded (`MAX_FILE_CHUNK_BYTES`), so the 1 MiB control cap is never approached.
- The receiver owns resume: it asks from the first chunk it still lacks, backed by `.alverchat-<id>-resume.part` and its sidecar index. A restarted download resumes; the sender starts from whatever `resume_from` says.
- The file is renamed into place only after the whole-file blake3 hash matches. A mismatch deletes every byte stored for that transfer, so a retry starts clean.
- File frames get no receipt: the protocol answers the `file_offer`, and everything after it is acked by `file_ack` or ended by `file_cancel`. A cancel is advisory and never awaited.
