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
