# Third-party notices

zen-minecraft-client includes generated registry data and test fixtures derived from the
following open-source projects. The zen-minecraft-client MIT license does not replace their
notices. Exact npm versions and registry integrity digests are recorded in
`reference/package-lock.json`.

## Locally recorded Java 1.21.11 diagnostics

`data/client_api/regular_click_*` contains factual registry/default-capacity,
native slot acceptance, PICKUP/SWAP outcomes and packet/hash encodings recorded
from the unmodified official Java 1.16.1 and 1.21.11 server JARs. The original
Java/Python exporters verify the original bundle/classpath hashes and call native
methods; no Minecraft binaries, mappings or method bodies are redistributed.
`regular_click_source.json` records the exact scope and source/output hashes.
The skeletal player/world context supplies inventory/default feature flags and
does not constitute a network, ownership or gameplay-mode test. Native capacity
facts correct the effective legacy warped_fungus_on_a_stick capacity from 64 to
1 during registry loading; the upstream `data/items.json` stays byte-for-byte
unchanged, including its existing harvest-audit source identity.
Regeneration and limits are documented in `docs/common-inventory-clicks.md`.

`data/client_api/inventory_transfer_*` separately records default QUICK_MOVE
routes, equipment slot acceptance/capacity, complete original menu outcomes,
legacy returned stacks/default constructor NBT, and original packet encodings
from those same unmodified official JARs. Original exporters supply an unspawned
player with actual Inventory/EntityEquipment/ServerPlayerGameMode; they do not
replace native algorithms or prove network/mode/ownership behavior. Source,
raw and output hashes and regeneration limits are recorded in
`inventory_transfer_source.json` and `docs/common-inventory-transfers.md`.

`data/java_1_21_11/outline_*` contains factual shape coordinates, native state
coverage and numeric raycast/rotation observations from a locally obtained Java
1.21.11 game. Original Java/Python tooling invokes native APIs; it is not copied
game source. Yarn 1.21.11+build.6 names identify the inspected methods. These data
do not relicense Minecraft or Yarn; no game JAR, mapping or decompiled source is
included. Exact source hashes, transformation and regeneration instructions are
in `outline_source.json` and `docs/player-targeting.md`.

`data/java_1_21_11/inventory_swap_packets.json` contains three factual native
packet encodings checked by the original `scripts/VerifyInventoryClick.java`
tool against `ClickSlotC2SPacket.CODEC`. The version is Java 1.21.11, with Yarn
1.21.11+build.6 method names. The development oracle uses the same package remap
and access-flag widening described by `outline_source.json`; native method bodies
are unchanged. No Minecraft binary, mappings or decompiled source is included.
The test source and validation scope are recorded in `docs/survival-inventory.md`.

`data/java_1_21_11/survival_foundation*.json` contains factual native dimensions,
attribute IDs/defaults/limits, cube admission, contact and packet observations.
The original `scripts/VerifySurvivalFoundation.java` invokes Java 1.21.11 APIs
using the same remap/access-only development oracle; no game method bodies are
copied. Source/output hashes and validation scope are recorded in the manifest
and `docs/survival-standing-context.md`.

`docs/evidence/survival-mining-*20261002*` contains locally recorded packet,
block/player observations and console diagnostics from an isolated official
Java 1.21.11 server. The driver is original test-private code and invokes native
player action packets; no Minecraft binary or decompiled code is redistributed.
The manifests record raw data hashes, transformation, failed attempt and scope,
including the separate native intent/result API comparison and its unvalidated
continuation boundary.

`docs/evidence/client-motion-*-20260929.json.gz` and their replay fixtures were
recorded by this repository's `packet_trace_probe` against an isolated official
Minecraft Java 1.21.11 server. The accompanying manifest identifies originals,
transformations, checksums and failed trials. Native state names/properties use
the pinned minecraft-data registry described below. These are local diagnostic
observations and original test tooling, not imported DustRoute source or fixtures.
The `client-players-*` records likewise come from two local clients on that
isolated server and include the failed/corrected native attribute-ID trial.
No Minecraft JAR or decompiled source is redistributed in this repository.
The inspected version and mapping identification are recorded in
`docs/client-piston-reconstruction.md`.

## User-supplied reference door diagnostics

`docs/evidence/client-reference-door-*-20260929*` records local observations of
Bobiloosky's One-Wide 3x3 Piston Door:
<https://www.planetminecraft.com/project/one-wide-3x3-piston-door-works-on-java-edition/>.
The user supplied world ZIP (SHA-256 recorded in the evidence manifest) was
previously inspected by DustRoute. Initial block coordinates/properties for
these local tests were read from DustRoute's `reference-3x3-observed-a-v1.json`
at commit `b1762b9`, translated by (-41900, 0, -900), and initialized on an
isolated server. No DustRoute implementation source or Minecraft binary is
included. These files are diagnostic records; Voxrig's MIT license does not
claim ownership of the circuit design or relicense the supplied world. Review
fixture redistribution attribution when preparing an upstream submission.

## minecraft-data 3.114.0

Upstream: <https://github.com/PrismarineJS/minecraft-data>

The Java 1.21.11 version adapter additionally uses `data/java_1_21_11/blocks.json`
(name, state ranges and complete property definitions), generated packet-ID
constants (including player registry IDs) in `src/versions/java_1_21_11/ids.rs`, and
`data/java_1_21_11/items.json` (native item ID, name and default stack size)
from the same exact package.
`data/java_1_21_11/collision_shapes.json` projects that package's block collision
shapes into native state IDs and AABBs. These are static collision shapes, not
graphical selection outlines or entity-dependent collision decisions.
`data/java_1_21_11/source.json` records the release, protocol, transformation and
generated block-data digest. These data do not depend on a running JavaScript client.

The files `data/blocks.json`, `data/items.json`, `data/materials.json`,
`data/entities.json`, `data/recipes.json`, `data/sounds.json`,
`data/block_state_ranges.json`, and `data/block_collision_shapes.json` contain
or are derived from Minecraft Java 1.16.1 registry data distributed by the
`minecraft-data-3.114.0.tgz` npm package. That exact package contains no
standalone license file. Its package metadata names
`Romain Beaumont <romain.rom1@gmail.com>` as author and declares the MIT
license; the upstream README also states MIT and cautions that individual
source data may require different terms.

`data/client_api/java_1_16_1_wire.json` contains the selected serverbound packet
IDs and schemas from upstream commit `886d159e4dc349d6b9204df2d17e1ea29e7b546e`.
It records the exact source URL and SHA-256 of the complete input protocol file.
This fixture is independently pinned for the common-client wire tests and is
covered by the same upstream MIT notice below.

MIT License

Copyright (c) Romain Beaumont and minecraft-data contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## prismarine-block 1.23.0

Source: <https://github.com/PrismarineJS/prismarine-block/tree/1.23.0>

`prismarine-block` was used by `reference/generate_fixtures.js` to construct
blocks supplied to the reference physics implementation. No
`prismarine-block` source code is embedded in zen-minecraft-client. The exact npm package
contains no standalone license file; its package metadata names
`Romain Beaumont <romain.rom1@gmail.com>` as author and declares MIT.

MIT License

Copyright (c) Romain Beaumont and prismarine-block contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## prismarine-physics 1.11.1

Source: <https://github.com/PrismarineJS/prismarine-physics/tree/1.11.1>

`data/prismarine_physics_fixtures.json` was generated by
`reference/generate_fixtures.js` using `prismarine-physics` 1.11.1,
`prismarine-block` 1.23.0, and `minecraft-data` 3.114.0. The notice below is
copied from the upstream `1.11.1` tag's `LICENSE` file.

MIT License

Copyright (c) 2020 PrismarineJS

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

The minecraft-data notice above also covers data/enchantments.json.
It uses the package dataPaths mapping for PC 1.16.1 to PC 1.13.2 shared
definitions. Exact path, package integrity and file SHA256 are recorded
in data/enchantments-source.json.

## Java 1.16.1 mining tool gate corrections v1

`data/harvest_gate_corrections.json` records name-based corrections to the
upstream `harvestTools` gate. The original upstream JSON files remain unchanged.
The manifest pins their SHA256 hashes and every corrected row's original names.
The loader rejects different source data, version, names or preimages.

Evidence: official Minecraft Java 1.16.1 server SHA1
`a412fd69db1f81db3f511c1463fd304675244077`, with official server mappings SHA1
`11120c39da4df293c4bd020896391fb9ddd6c2ba`.
The verification probe enumerated registered block states and called
`requiresCorrectToolForDrops` and `Item.isCorrectToolForDrops` directly.
The probe and audit are retained in the consuming zen repository as
`tools/C17HarvestOracle.java` and `docs/analysis/C17_HARVEST_LOOT_AUDIT.md`.

These corrections describe only the mining tool gate. They do not guarantee a
loot item, replace loot tables, change mining speed, or make unbreakable blocks
breakable. Silk Touch, block-state conditions and special destruction behavior
remain separate. Descriptor revision 3 distinguishes these effective definitions
from the original registry. Upstream PR #407 is background evidence of the
netherite additions, not the authority used to infer corrected tool membership.

The native correction and dry-movement method/codec observations in
`data/java_1_21_11/position_corrections.json` and `dry_movement.json` use original
Java callers of unchanged target-version method bodies. Their source hashes,
mappings and scope are recorded in `dry_movement_source.json`. No Minecraft
class files or method bodies are distributed. See `docs/survival-motion-controls.md`.

## Common-client native Java diagnostics

`data/client_api/java_1_16_1_dry_movement.json` and
`data/client_api/legacy_targeting_oracle.json` contain factual numeric observations
from the unmodified official Java 1.16.1 server JAR. Original Java callers invoke
native movement, view-vector, outline and clipping methods. The targeting probe
initializes only native ClipContext data fields for an empty collision context;
it does not replace native traversal or clipping method bodies. Official server
mapping names identify the inspected methods. Source/verifier/output hashes and
scope are retained in their adjacent source records and the common-survival docs.
No game JAR, mappings or decompiled source is redistributed.

`data/client_api/storage_outlines-*.json` and `storage_outline_rays-*.json.gz`
are factual numeric observations from unchanged official 1.16.1 and 1.21.11
state-only shape and clip methods. Original `scripts/ExportStorageOutlines.java`
enumerates complete storage properties and calls native `BlockGetter.clip`.
The source record pins input/output/tool hashes and scope. Animated or world-
dependent block entity shapes are excluded; no native method bodies are shipped.

`data/client_api/common_native_evidence.json` contains local common-Client and
independent server RCON observations from sequential official vanilla 1.16.1 and
1.21.11 trials. Original Rust/Python test tooling and source hashes document the
limited scenarios. These observations do not relicense Minecraft.
