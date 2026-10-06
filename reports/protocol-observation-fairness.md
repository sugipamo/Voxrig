# Java 1.16.1 Packet / Observation Scheduling

## Reproducer

On main `af91cad65ebb441f9ab775c6e95747cdd8fc07a5`, the packet reader's
biased select prioritizes both observation queues over the pinned packet read.
With a nonempty observation queue, neither a ready packet nor the original
packet timeout is polled until those higher-priority requests drain.

Three deterministic local tests fail on that baseline:

- Eight ready coherent captures all return without ready KeepAlive and health
  packets being applied. The eventual KeepAlive response reaches a mock TCP
  server with the exact original payload, but only after the captures drain.
- The same happens with eight traversal movement-facts captures.
- A paused-clock packet deadline that expired at 100 ms is checked only after
  eight queued captures complete. The deadline is preserved, but not polled.

These finite fixtures demonstrate scheduling starvation, not a production-scale
keepalive timeout. A continuously replenished queue can keep the prioritized
branch ready; the actual request pressure in the historical incident is unknown.

## Change And Boundary

After each capture the reader polls the **same** pinned packet future once,
without waiting for more bytes. A ready packet or expired deadline is handled
before the next capture. Pending partial reads retain consumed bytes and the
original deadline. Ready cancellation retains priority. Captures still get an
opportunity ahead of a ready packet backlog.

No protocol version, public API, reply payload, observation identity, transport
retry or timeout duration changes. No automatic reconnect or fabricated ACK.
This is not a wall-clock guarantee for an individually slow capture, packet
application, writer, shared lock, or a host that is not being scheduled. It also
does not change priority between the two observation request queues.

The regression suite checks exact TCP KeepAlive echo, fresh health during both
capture backlogs, preserved capture opportunity, expired-deadline polling and
ready-cancel priority. Existing partial-packet and packet-deadline regressions
must continue to pass.

## Historical Incident Is Not Yet Attributed

Voxrig issue #5 records nine server-side timeouts in a ten-worker Evolto run on
2026-10-06 at 02:05:04-02:05:07 UTC. No packet trace establishes that this
scheduling bug caused those timeouts. Concurrent build load on the shared host
remains another possible contributor. Fixing the isolated bug is not evidence
that the incident's root cause has been proven or that long-running stability
is guaranteed.

The unpatched production controller subsequently timed out all eleven clients
at 06:16:58-06:17:15 UTC. It fail-closed at 06:17:38 UTC after unconfirmed
cancellation, preserving holds; the Minecraft server remained active. The
single-job, low-priority test build also had an LLD crash. Their overlap does not
prove a common cause. No patched production run has been performed.

## Opt-In Timing Evidence

The original scheduling fix was merged as PR #6 (`4aba2ac`). Timing diagnostics
are a separate follow-up, PR #8; they were not part of that merge.

Set `VOXRIG_TRACE_PROTOCOL=1` before starting a diagnostic process to emit
`voxrig_protocol_timing` JSON lines to stderr. Logging is off by default and
bounded to 65,536 records per process, followed by one `capacity_exhausted`
marker. Do not interpret missing records after that marker as missing packets.

- `keepalive_frame_decoded` records the connection generation, exact KeepAlive
  identity and Unix timestamp before coherent-gate admission. It means a full
  frame was decoded, not that the TCP bytes just arrived or that prior backlog
  was absent.
- `keepalive_reply_write_completed` or `keepalive_reply_failed` correlates
  by generation/identity and includes monotonic elapsed milliseconds from
  decoded-frame handling through gate wait, actor queue and socket write.
  A completed write is NOT proof of peer receipt or acceptance. Failure includes
  a bounded error string and may be an admission rejection without a write.
- `slow_capture` records coherent or movement captures taking at least 100 ms,
  including waits, sequence and success. It emits only after the capture ends;
  an unfinished capture or whole-process scheduling stall remains unmeasured.

No raw payloads, inventory, world data or user chat are logged. These records
do not alter packet admission, timeouts, replies, reconnect or replay policy.
Use an independent server log/packet trace and host scheduling evidence to
separate absent client receipt, delayed handling, failed write and peer timeout.
Diagnostic stderr must be drained; logging itself can add timing overhead.

Run the local scheduling tests with:

```bash
cargo test --locked --lib protocol_fairness_tests -- --test-threads=1
cargo test --locked --lib generic_read_loop_preserves_one_partial_packet_future_deterministically
cargo test --locked --lib packet_deadline_tests -- --test-threads=1
```

## Validation On 2026-10-06

- Baseline: all three original scheduling regressions fail as described above.
- Fixed all-target test run: 367 passed, 8 opt-in tests ignored, none failed;
  includes four new scheduling tests and the existing partial-read/deadline tests.
- Doc tests: four passed, including two compile-fail examples.
- `cargo fmt --all -- --check`, all-target Clippy with `-D warnings`, and
  `cargo package --locked --allow-dirty --list` passed.
- Compilation was restricted to one low-priority job on the production host.
  The existing Evolto release remained connected, without a service restart or
  a new transport failure. It **does not contain this patch**. The isolated
  fixtures do not certify a patched ten-bot real-server endurance run.

Follow-up after adding opt-in timing evidence: 369 all-target tests passed,
8 opt-in tests ignored; four doc tests and warnings-denied all-target Clippy
passed. Mock-server trace records were parsed to check generation/KeepAlive
pairing, exact echo, a measured 100+ ms coherent-gate wait and reply rejection
without barrier bypass. Production stayed stopped, retaining all holds; these
checks do not attribute the historical server timeout.

After incorporating concurrently merged main `344018c`, all-target validation
passes 372 tests (8 opt-in ignored), thirteen doc tests and warnings-denied Clippy.
The PR #8 diff remains limited to these diagnostics, regression tests and docs.
