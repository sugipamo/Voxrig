# R0 baseline

> 過去の派生版の実装・検証記録です。現行の名称・統合方針は[client API設計](public-client-api.md)を参照してください。

Date: 2026-08-29 UTC

## Identity

- upstream source: `https://github.com/sugipamo/Voxrig.git`
- source commit: `6434b2cd8d7328d397b34b0151660a9882d844fc`
- inherited package: `voxrig 0.1.0`
- `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- `cargo 1.97.1 (c980f4866 2026-06-30)`
- runtime: OpenJDK 8u502, Linux x86_64
- real-server jar SHA-256:
  `2782d547724bc3ffc0ef6e97b2790e75c1df89241f9d4645b58c706f5e6c935b`

The source tests were first run before R0 protocol/resource hardening. The
candidate tests below include that hardening and are the gate result.

## Source baseline

| Check | Result |
| --- | --- |
| `cargo test --all-targets` | pass, 60 tests |
| `cargo test --doc` | pass, 1 doctest |
| `cargo fmt --all -- --check` | not initially runnable; rustfmt component absent |
| `cargo clippy --all-targets -- -D warnings` | not initially runnable; clippy component absent |

## R0 candidate

After installing the standard rustfmt and clippy components and adding
malformed/oversized protocol and cache-boundary regressions:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo test --all-targets` | pass, 72 tests |
| `cargo test --doc` | pass, 1 doctest |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo test --lib -- --test-threads=1` | pass, 72 tests |
| `git diff --check` | pass |

Added regression boundaries cover VarInt truncation/overrun, oversized and
truncated frames, write body limit, compression threshold mismatch, trailing
compressed data, entity/map/chunk insertion rejection at their individual
limits, aggregate session-cache limit detection, oversized custom payload
suppression, and event receiver lag. The inherited aggregate limit check runs
after packet-specific state application; this baseline does not claim an
aggregate rollback guarantee. R1 must make an over-limit actor generation
terminal before any later coherent observation can expose partial state.
Existing tests cover NBT depth/node/array limits, chunk palettes and packed
arrays, registry counts, map payload geometry, and physics metrics bounds.

## Real-server probe

The Host started an isolated Minecraft Java Edition 1.16.1 offline-mode flat
server bound to `127.0.0.1:25566` and ran:

```text
MC_PORT=25566 cargo run --release --features native --example api_surface_probe
```

Result: pass.

```text
commands=20 tags=144 recipes=859 chunks=121 shared_sections=66 live_sections=1 fluid=None climbable=false
```

Both probe clients logged out, the server completed its normal save/stop
sequence, and no probe result was used as a substitute for unit or contract
coverage.

## R0 remaining gate

The Zen-side exhaustive boundary inventory is
`docs/RUST_CLIENT_CONTRACT_MAPPING.md` in the Zen repository. It classifies
every Input observation envelope field, nested fact group, primitive variant,
result, and candidate provider as `direct`, `transform`, `missing`, or
`responsibility_conflict`.

R1 remains blocked until the adoption/lifecycle Technical Decisions are
recorded and independently audited. In particular, this baseline does not
authorize treating domain revisions as coherent observations, treating
transport end as logout success, or moving TD-407 transaction ownership
without an explicit replacement decision.
