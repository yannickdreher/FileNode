# Transfer limits, push and interrupted connections

## Server and user limits

JMAP discovery reads `urn:ietf:params:jmap:core.maxSizeUpload` in bytes and `maxConcurrentUpload`. Settings show the effective upload size and parallelism after sign-in. Server limits are refreshed when the account reconnects.

- `max_file_bytes = 0` (new-profile default): use the advertised upload limit. If it is absent, use a 1 GiB fallback. A positive value is an additional user file-size cap; the lower upload limit wins. The user cap also limits downloads. Existing profiles retain their explicit value, including the previous 1 GiB default; set it to zero for automatic mode.
- `parallel_uploads = 2` by default, configurable from 1 to 16. Actual concurrency is bounded by this preference and the server's advertised maximum. Missing/invalid concurrency discovery falls back to one upload.
- The desktop edits the optional size cap in MiB. Size, concurrency and exclusions remain editable while syncing. Save settings validates and persists the changes, stops and joins the previous worker, then continues automatically. Invalid settings leave the current worker running.
- Quota, permissions, proxy limits, free disk space and server policy can independently reject a transfer. A discovery limit does not guarantee that every upload below it will succeed.

An oversized local file produces a persistent per-file warning with its size and effective limit. It is not hashed/uploaded or interpreted as a deletion; other files continue. Oversized downloads under an explicit user cap are also preserved/skipped. Parent operations that would remove or move skipped data are blocked. A later successful transfer updates the same activity row. Headless `--once` returns an error if files were skipped by the size limit.

## Parallel payloads, ordered commits

Only a bounded batch of consecutive upload actions is prepared at once. Each worker verifies a stable local snapshot and streams a blob. FileNode create/update, history preservation and SQLite baseline/journal completion remain serial and use `ifInState`. Completed payload bytes mean **awaiting confirmation**; only a successful state/baseline update marks a file synchronized. Concurrent server changes still fail the conditional operation instead of overwriting unseen edits. Failed or abandoned temporary blobs are left for server TTL cleanup, never published as partial FileNodes.

Upload and download payload-rate schedules are shared per account, including across simultaneous uploads and selected folders. More connections do not multiply the configured KiB/s limit. Staging consumes temporary disk space and the selected parallelism also affects disk I/O. Downloads remain serial. HTTP/TLS overhead, kernel buffers and proxy behavior are outside these payload limits.

## WebSocket push

If discovery advertises `urn:ietf:params:jmap:websocket` with `supportsPush: true`, the client negotiates the `jmap` subprotocol and enables `FileNode` notifications. Credentials are only sent to a validated same-origin `wss://` URL without userinfo, query or fragment. API requests and payloads continue over HTTPS; this is not a general replacement of the API transport.

Push is an invalidation hint, not the source of truth. Only the selected account's FileNode changes wake remote reconciliation. A new/reconnected channel also schedules a check to recover missed notifications. Polling and periodic full verification remain active, including when the server does not offer push, the endpoint is rejected or the channel is unavailable. Push-only checks reuse the local hash cache; they do not force a full rehash. Sleep/energy-saver policy closes or defers the channel. Idle connections use ping/pong and reconnect with bounded backoff. Healthy push does not bypass failed-folder retry deadlines.

## Interrupted uploads and downloads

Downloads retain staged partial content and resume with validated HTTP Range responses. A dropped **upload restarts that file from byte zero** from a freshly verified snapshot; arbitrary-offset/chunked upload resumption is not implemented. Other completed baselines remain committed. Ambiguous FileNode acknowledgments are reconciled by remote identity/state on retry. A lost upload acknowledgment can leave an unreferenced temporary blob until its server TTL expires.

The HTTP client reconnects after broken sockets; desktop connection failures re-discover the account and refresh credentials/session limits. Folder failures retry from 5 seconds to 5 minutes; push reconnects from 1 second up to 60 seconds. Checks remain periodic during push outages. There is no native network-change subscription or immediate connectivity-triggered retry, and Retry-After/jitter are follow-up work. Sleep/resume, Wi-Fi/VPN switching and real-server faults still require native acceptance testing beyond local simulations.

References: [JMAP session limits](https://www.rfc-editor.org/rfc/rfc8620.html#section-2), [binary upload](https://www.rfc-editor.org/rfc/rfc8620.html#section-6.1), [JMAP over WebSocket](https://www.rfc-editor.org/rfc/rfc8887.html).
