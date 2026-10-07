# Java 1.16.1 Play Reader Throughput

This internal change addresses two measured scheduling dependencies in the
Java 1.16.1 adapter. It does not change the public API or other adapters.

## Boundaries

- Play compression is negotiated during login and remains fixed for that
  connection. The reader snapshots it once instead of locking the outgoing
  packet writer before every incoming frame. The writer still owns outgoing
  encoding and ordering.
- Every applied packet still checks the same eight caches against
  `max_cached_records`, with the original counts and saturation behavior.
  The short cardinality reads first use `RwLock::try_read`; if unavailable,
  they fall back to the original awaited read. Tokio's queued writer fairness
  is retained.
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

`cargo test --lib -j 1`: 387 passed, zero failed, eight existing tests ignored.
The existing partial-frame, deadline, capture fairness and aggregate fail-close
tests remain included. Controlled tests establish these specific dependencies
are removed; shared-runtime load and every deployment timeout are not thereby
proven resolved.
