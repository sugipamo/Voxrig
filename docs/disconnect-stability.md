# Disconnect and in-flight protocol replies

Java 1.16.1 previously classified a protocol reply rejected by the disconnect
operation barrier as a reader failure. A KeepAlive or teleport acknowledgement
already in flight could therefore change `Disconnecting` to `Unknown`.

The actor now attaches a private typed reason to pre-write protocol admission
rejection. The reader continues only when both that reason and its current
lifecycle are `Disconnecting`. It consumes subsequent frames until actual
transport termination. Rejection does not count as a delivered reply and does
not by itself commit a successful disconnect. Malformed frames, genuine write
failures and other actor/reader errors retain their existing classification.

The deterministic TCP regression sends a KeepAlive after receiving the client's
actual write-half EOF. The common disconnect must remain pending while the
server retains its write half. Releasing it gives a normal disconnect; sending
an overlong frame-length VarInt instead must fail with `Unknown`. The former
case reproduced issue #20 before the change. A separate actor test verifies
that admission rejection writes no packet and is distinct from a real protocol
write failure, which still becomes `Unknown`.

Real connections used checksum-verified official vanilla 1.16.1 and 1.21.11
servers, offline loopback only, view distance 6 and max players 8. With the
`native` feature disabled, each version completed ten fresh-name cycles:
wait until ready, twice wait one second and read player/chunk state, disconnect,
then verify server-side player removal. All twenty cycles passed. The first
legacy cycle still had only 108 of 225 chunks at disconnect; later cycles reused
warm server chunks. These are regression checks, not evidence that every cycle
forced the race. The TCP regression controls that interleaving directly.

Results and original report hashes are in
[disconnect-stability-20261008.json](evidence/disconnect-stability-20261008.json).
Reproduce the real connection checks with a Java runtime compatible with both
servers:

```sh
cargo build --locked --example connection_stability_probe
python3 scripts/run_connection_stability.py \
  --binary target/debug/examples/connection_stability_probe
```

The runner downloads and verifies official jars, owns disposable servers and
records reports under `.local/climbing/live`. `--jars PATH` reuses verified jars;
`--version 1.16.1` selects a single version. It stops only its own processes.
