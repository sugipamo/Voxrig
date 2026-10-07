# Java 1.16.1 Play Reader Throughput

This internal change addresses two measured scheduling dependencies in the
Java 1.16.1 adapter. It does not change the public API or other adapters.

## Boundaries

- Play compression is negotiated during login and remains fixed for that
  connection. The reader snapshots it once instead of locking the outgoing
  packet writer before every incoming frame. The writer still owns outgoing
  encoding and ordering.
- A single standard Tokio `BufReader` survives both login and play. It amortizes
  small wire reads and retains bytes prefetched across compression negotiation
  and the login-success boundary. Frame/decompression allocation limits are
  unchanged.
- Every applied packet still checks the same eight caches against
  `max_cached_records`, with the original counts and saturation behavior.
  The short cardinality reads first use `RwLock::try_read`; if unavailable,
  they fall back to the original awaited read. Tokio's queued writer fairness
  is retained.
- Frequent relative-move, rotation, ground, head-yaw, velocity and teleport
  entity updates use the equivalent fair `try_write`/async fallback. Local
  entity-ID checks use `try_lock`/async fallback. Other handlers, physics and
  public snapshot locks are unchanged.
- The reader yields after every 32 completed packets, outside all state guards.
  Captures and cancellation are still polled on each frame, not only each batch.
  One registered cancellation future survives packet application and batch
  yields so `notify_waiters` cannot be lost between successive selects.
- Socket reads, coherent-state locking, handlers and captures still participate
  in cooperative scheduling. This does not unconstrain the entire reader,
  bypass packets or drop backlog to reach KeepAlive sooner.
- Partial-frame progress, packet deadlines, capture fairness, operation
  barriers and connection-unknown fail-close behavior remain unchanged.

## Evidence And Tests

[Issue #5](https://github.com/sugipamo/Voxrig/issues/5#issuecomment-6031605038)
records an isolated ten-worker plus spectator Director reproduction using
Evolto `42f479a`, Golemkit `8cef0db` and baseline Voxrig `686f414` with Tokio
1.53.1. Of 526,584 measured cache acquisitions, 3,918 yielded only with
exhausted cooperative budget. Those acquisitions accounted for 99.9531% of
aggregate acquisition elapsed time, summed across eleven readers. No pending
poll with available budget was observed. This is not a claim that 99.95% of
all processing is in cache reads or that every timeout has the same cause.

Added regression tests cover:

- A free cache read and the complete eight-cache check with exhausted budget.
- Both an already held writer and a writer queued ahead of the new reader.
- External inventory growth detected after a non-growing incoming packet.
- Two successive frames while the outgoing writer is held, for compression
  disabled, enabled below threshold and enabled above threshold.
- Login/compression/play frames delivered together, without losing prefetched
  data; a buffered partial frame interrupted by a coherent capture.
- Free update locks with exhausted budget, contended/queued update owners,
  and a ready backlog yielding at exactly 32 frames with cancellation at the
  batch boundary.

`cargo test --lib -j 1`: 392 passed, zero failed, eight existing tests ignored.
The existing partial-frame, deadline, capture fairness and aggregate fail-close
tests remain included. Controlled tests establish these specific dependencies
are removed; shared-runtime load and every deployment timeout are not thereby
proven resolved.

The first two isolated fix trials did still time out (about 106 and 107 seconds).
The first had 3,736,632 cache acquisitions with no pending polls, but packet
application/coherent-gate delays remained. Adding buffering alone also did not
resolve that trial's timeout. These negative results motivated the fair
hot-update fast paths and bounded batch scheduling; they must not be hidden or
presented as evidence of a complete timeout fix.

## Diagnostic Reader Progress

An additional temporary investigation probe requires both
`VOXRIG_TRACE_PROTOCOL=1` and `VOXRIG_TRACE_READER_PHASES=1`. Both are opt-in;
reader progress is disabled by default. It samples once per second, with the
existing process-wide 65,536-record bound. Each record identifies the SDK
generation, current phase/packet, phase age, cumulative phase time, decoded and
applied counts, capture starts/completions and actual packet-read poll counts.
Coherence-gate waiting is separate from packet handler execution. An unfinished
await remains visible, including in the final record when the reader is dropped.
Dropping the reader aborts its reporter; the reporter never acquires gameplay
locks. It forwards exactly one poll to the same pinned timeout/read future, so
partial-frame retention, deadline and packet order do not change.

The enabled probe adds per-frame clock/mutex work and a small reporter task.
Measurements therefore are not a controlled uninstrumented A/B comparison or a
proven fix. It neither drops EntityStatus traffic nor bypasses the coherence gate.
