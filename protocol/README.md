# THMR local control protocol 1

This is a local desktop protocol, not the public radio or account API. A host owns one player and one native media session. Frontends do not decode audio.

The per-user `instance.json` contains a random loopback port, protocol version, instance ID and a 256-bit bearer credential. Never print or publish that file. HTTP requests and WebSocket upgrades require `Authorization: Bearer …`; browser Origin headers are rejected. The host does not enable CORS. TCP is bound exclusively to `127.0.0.1`.

- `GET /v1/state`, with `X-THMR-Protocol: 1`: full confirmed snapshot.
- `POST /v1/command`: `{ "protocolVersion": 1, "command": { "command": "set_volume", "value": 35 } }`.
- `GET /v1/events` WebSocket upgrade, with `X-THMR-Protocol: 1`: sends a complete snapshot on connection and whenever the revision changes. Intermediate revisions may be coalesced; every message is independently usable. Reconnect always replaces the local snapshot.
- `GET /v1/audio` WebSocket upgrade, with the same authorization and protocol header: independent, coalescible audio-feature snapshots. `capabilities.syncedAudio` advertises this endpoint; older hosts omit the capability. Frontends must not fall back to a second radio request.

Commands: `play`, `pause`, `stop`, `toggle`, `set_volume` (`value` 0–100), `set_source` (`source`: `auto`, `source1`, `source2`), `set_profile` (`profileId`: enabled radio profile), `refresh`, `test_sources`, `shutdown`, `prepare_update`. Unknown command fields are rejected.

HTTP errors return `{ "error": "…" }`. Unsupported protocol versions return 409. Commands are confirmed once by HTTP; clients must not automatically retry non-idempotent commands such as toggle. Each snapshot includes `protocolVersion`, product `version`, `buildNumber`, `instanceId`, monotonic `revision`, `phase`, `userPaused`, volume, selected source/profile and raw `stationStatus` compatible with the radio status model. Audio and metadata errors are separate.

Live playback has no seek, previous or next capability. Pause stops downloading audio; play rejoins the current live edge. Source/profile changes preserve pause intent. `stop` does not terminate the host; `shutdown` does.

`test_sources` starts bounded anonymous probes and reports `sourceTesting`/`sourceMeasurements` in subsequent snapshots. It preserves an explicitly selected source. Probes never forward playback credentials.

`prepare_update` publishes a final snapshot with `shutdownReason: "update"`, then stops audio and releases the endpoint and instance lock. Attached TUI clients leave on this notification; the GUI updater waits for the host to exit before replacing the installation. Other clients must treat a new `instanceId` as a new session and discard their previous revision. TUI `q` detaches without stopping the host. Explicit GUI exit sends `shutdown` and waits for the host to release its endpoint before closing; minimizing/hiding the GUI keeps playback running.

`THMR_RUNTIME_DIR` and `THMR_CONFIG_DIR` override user paths for isolated development/testing. They must point to directories owned by the current user. Configuration is non-secret TOML; credentials never belong in source URLs, process arguments or logs.

## Playback-synchronized audio features (audioVersion 1)

The host opens one playback stream and decodes MP3 once into PCM. The same PCM feeds libmpv and the analyzer. Features carry the final sample position of their FFT window; the host queues them ahead, then publishes only features due at libmpv's `audio-pts`. Consumed features are retired. Analysis is bounded to three seconds ahead plus at most one MP3 frame. This is independent of metadata polling and bounded source-quality probes.

Each audio message contains `audioVersion`, `instanceId`, `epoch`, `sequence`, `active`, `sampleRate`, `playbackSample`, optional `feature`, and `queue` statistics. A feature contains `endSample`, `display` (48 bands and `maxHz`), stereo RMS/peak `levels`, and cumulative musical event counters. `feature.endSample <= playbackSample` is mandatory. These positions are on the same epoch's input PCM timeline, not Unix time or downloaded byte counts. Frontends render the latest delivered due feature and retain local arrival time only to detect stale transport.

Pause, stop, reconnect, source changes and profile changes invalidate the old content epoch. Resume rejoins the live edge in a new epoch. Never compare samples or musical counters across `(instanceId, epoch)`. Inactive frames carry no usable feature. Clients clear old display data on epoch changes and baseline event counters on resumed availability, so stale beats are not replayed.

The host publishes audio independently at up to 50 Hz; normal state snapshots and metadata keep their existing cadence. Slow subscribers may miss intermediate frames: counters remain cumulative within an epoch, and every message stands alone. Static TUI mode and disconnected clients cancel their local audio subscription; host decoding and bounded ahead analysis remain part of playback.

For local measurements only, `THMR_AUDIO_TRACE_DIR` records per-epoch source PCM, decode timestamps, request attempts and selected-feature positions. `THMR_TUI_TRACE_DIR` records terminal submission timestamps and content positions. Both are off by default and record no authentication headers or radio URLs. These files can contain audio; they are local developer evidence, not diagnostics uploaded by the application.
