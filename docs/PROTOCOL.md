# Protocol v1

Every control frame uses an envelope:

```json
{"v":1,"id":"<uuidv7>","type":"message","from":"<peer-id>","to":"<chat-id>","hlc":{"physical_ms":0,"logical":0},"payload":{}}
```

Unknown envelope types are designed to be ignorable. Binary payload framing for file chunks remains to be implemented. The current core defines typed envelope and HLC primitives but does not yet open a listener or exchange frames.
