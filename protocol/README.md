# THMR local control protocol 1

This is a local desktop protocol, not the public radio or account API. A host owns one player and one native media session. Frontends do not decode audio.

The per-user `instance.json` contains a random loopback port, protocol version, instance ID and a 256-bit bearer credential. Never print or publish that file. HTTP requests and WebSocket upgrades require `Authorization: Bearer …`; browser Origin headers are rejected. The host does not enable CORS. TCP is bound exclusively to `127.0.0.1`.

- `GET /v1/state`, with `X-THMR-Protocol: 1`: full confirmed snapshot.
- `POST /v1/command`: `{ "protocolVersion": 1, "command": { "command": "set_volume", "value": 35 } }`.
- `GET /v1/events` WebSocket upgrade, with `X-THMR-Protocol: 1`: sends a complete snapshot on connection and whenever the revision changes. Intermediate revisions may be coalesced; every message is independently usable. Reconnect always replaces the local snapshot.

Commands: `play`, `pause`, `stop`, `toggle`, `set_volume` (`value` 0–100), `set_source` (`source`: `auto`, `source1`, `source2`), `set_profile` (`profileId`: enabled radio profile), `refresh`, `test_sources`, `shutdown`, `prepare_update`. Unknown command fields are rejected.

HTTP errors return `{ "error": "…" }`. Unsupported protocol versions return 409. Commands are confirmed once by HTTP; clients must not automatically retry non-idempotent commands such as toggle. Each snapshot includes `protocolVersion`, product `version`, `buildNumber`, `instanceId`, monotonic `revision`, `phase`, `userPaused`, volume, selected source/profile and raw `stationStatus` compatible with the radio status model. Audio and metadata errors are separate.

Live playback has no seek, previous or next capability. Pause stops downloading audio; play rejoins the current live edge. Source/profile changes preserve pause intent. `stop` does not terminate the host; `shutdown` does.

`test_sources` starts bounded anonymous probes and reports `sourceTesting`/`sourceMeasurements` in subsequent snapshots. It preserves an explicitly selected source. Probes never forward playback credentials.

`prepare_update` publishes a final snapshot with `shutdownReason: "update"`, then stops audio and releases the endpoint and instance lock. Attached TUI clients leave on this notification; the GUI updater waits for the host to exit before replacing the installation. Other clients must treat a new `instanceId` as a new session and discard their previous revision. GUI and TUI closure normally detaches without stopping the host; `shutdown` is the explicit global exit.

`THMR_RUNTIME_DIR` and `THMR_CONFIG_DIR` override user paths for isolated development/testing. They must point to directories owned by the current user. Configuration is non-secret TOML; credentials never belong in source URLs, process arguments or logs.
