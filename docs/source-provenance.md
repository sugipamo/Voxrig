# Source provenance

## Rebuild base

`zen_minecraft_client` starts from the complete Git history of Voxrig at the
following immutable source revision:

- upstream: `https://github.com/sugipamo/Voxrig.git`
- commit: `6434b2cd8d7328d397b34b0151660a9882d844fc`
- upstream package: `voxrig 0.1.0`
- Minecraft target: Java Edition 1.16.1, protocol 736, offline mode
- minimum Rust version declared by upstream: 1.85

The rebuild does not promise Voxrig API, module, or semantic-version
compatibility. Retaining history is for attribution, provenance, and audit;
it does not make upstream responsible for later Zen-specific changes.

## License preservation

The upstream source is MIT licensed. The root `LICENSE` remains authoritative
for the inherited source and must remain in distributions containing a
substantial portion of it.

`THIRD_PARTY_NOTICES.md` must also remain present. It records the exact
third-party sources used for embedded Minecraft registry and physics fixture
data:

- `minecraft-data 3.114.0`
- `prismarine-block 1.23.0`
- `prismarine-physics 1.11.1`

Their resolved npm versions and integrity hashes are retained in
`reference/package-lock.json`. Regenerating or replacing embedded data requires
updating the notice and recording the generator input revision before merging.

## Reproducibility boundary

The inherited `Cargo.lock`, embedded `data/` files, generator sources under
`reference/`, and protocol coverage document are part of the R0 baseline.
Changes to them require a review that distinguishes generated-data changes
from client implementation changes.

The source revision above is the comparison baseline. Later baseline reports
must record the tested commit, Rust toolchain, operating system, server jar
identity when applicable, and whether the result came from a unit/contract
test or a real-server probe.
