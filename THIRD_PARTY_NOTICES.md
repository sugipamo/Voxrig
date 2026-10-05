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


The item-component registry and codec fixtures in
`data/client_api/item_components-1.21.11.json`,
`item_component_cases-1.21.11.json`, and their source/request metadata are
original-tooling outputs from the unmodified official Java 1.21.11 server.
They record registry facts and encoded codec inputs/outputs, not Minecraft
method bodies or redistributed game JARs. The own reflection wrappers and
input hashes are identified by `item_component_source.json`; original JARs,
classpath libraries, mappings and bytecode inspection logs remain local.

`data/client_api/item_data_native_evidence.json` records actual common-Client
observations and independent RCON facts from sequential unmodified vanilla
1.16.1/1.21.11 trials. Recorded wire values, run-time input digests and earlier
failed attempts document the limited named-item/custom-data reception scope.
Original game binaries, libraries, method bodies and disposable worlds are
excluded from the package; these facts do not relicense Minecraft.

`item_component_schema-1.21.11.json` records factual stream-codec compositions,
resolved recursion and original dispatcher branches observed by the own
`ExportItemComponentSchema.java` reflection tool. The schema contains encoded
field boundaries, not decompiled method bodies. Component fixture generation
now invokes original vanilla resource, registry and tag loaders and records
actual native item-prototype values; fixture registry IDs are not universal
bindings for arbitrary servers. `item_data_complex_native_evidence.json` retains
separate original live observations and the earlier failed own-marker check.

`data/client_api/nbt_semantics-*.json` and their requests/source metadata record
native decoding, equality and pure modern persistent-codec CRC32C facts from
unmodified official 1.16.1/1.21.11 JARs. The own Java/Python wrappers also record
the absence of default custom data in all 1,505 pinned modern item prototypes.
These are factual outputs, not native method bodies or complete inventory hashes.
Original JARs, libraries, mappings and inspection logs remain local; these facts
do not relicense Minecraft.

`data/client_api/item_properties-*.json` and their requests/source metadata are
factual outputs of unchanged original item constructors, prototypes, item stream
codecs and property getters. Own wrappers enumerate 975 legacy/1,505 modern item
defaults (including AIR), retain 3,490 modern prototype component values and
decode 1,428 legacy/1,288 modern candidate data cases. Prototype registry bytes
belong to the pinned vanilla oracle context, not arbitrary live connections.
No game binaries or method bodies are included, and these facts do not establish
full item semantics, inventory hashes, slot policy or action permission.

`data/client_api/item_properties_native_evidence.json` retains actual common
property interpretations from original item bytes and pinned prototypes in both
versions/modes, with original item receipt provenance, independent RCON and
readonly frames. Captured execution-input/raw-output hashes and an explicitly
undelivered modern frame after requested disconnect limit the observations;
no game binaries or method bodies are included.

`data/client_api/nbt_semantics_native_evidence.json` retains actual typed metadata,
original item bytes and source ordinals from sequential vanilla trials in both
versions/modes, with independent RCON and readonly frames. Run-time input and
raw-output hashes bind these limited observations; game binaries are excluded.

`data/client_api/item_semantics-*.json.gz` records factual independently decoded
native item/component comparisons, prototype neutrality, typed persistent hash
inputs and unchanged hashed-stack codec outputs. Own observation wrappers
forward factories/builders to original HashOps and compare to unwrapped encoders.
The supplied HashGenerator composes unchanged typed encoders and original Guava
native-key caching at the mapped capacity; it is not live ServerPlayer cache
or synchronizer evidence. Source/request/JAR/mapping/classpath/raw-output hashes
are recorded in `item_semantics_source.json`. Original binaries, mappings and
method bodies remain local; these facts do not relicense Minecraft.

`component_value_rules-1.21.11.json`, its compressed cases and source record
retain factual enum IDs/names/factory aliases and fixed scalar/NBT decoder
outputs from unchanged original codecs, getters, NbtIo and NbtOps. Native enum
factory/math bytecode inspection stays local; no method bodies are distributed.
Original JAR/mappings/classpath/tool/raw/final digests bind the limited primitive
scope. Typed stream fields are not full native component/text equality, resolved
live registries, cached item hashes or gameplay support; these facts do not
relicense Minecraft.

`component_normalization_rules-1.21.11.json`, its compressed cases and source
record contain factual forward-codec identities, original 1.16.1/1.21.11 resource
ID constructor/stream outputs and adverse modern text parsing/encoding/equality
results. Text component comparisons and original ItemStack data/matches results
remain an oracle for future runtime semantics; conversion/encoding failures are
identified separately from original stream rejection. UTF-16 diagnostic inputs
are retained as code units where needed. Original JAR/mapping/classpath/tool/input/
raw/final digests bind these facts. Native method bodies and inspection logs stay
local, and no game binaries are distributed or relicensed. These are not complete
runtime text semantics, inventory authority or live-server cache evidence.

`text_color_rules-1.21.11.json`, `text_core_cases-1.21.11.json.gz` and their
source record contain observed original text contents/style fields, numeric
wrapper classes, component comparisons, native named colors and the running
JDK Character.digit(char,16) grammar. Owned standalone tools invoke unchanged
original codecs/getters; original numeric bytecode inspection remains local.
The shared field model covers a limited dependency-free comparison scope;
selectors, profiles, URI, dialog/item/entity references, full constructors,
persistent encoding and live server caches remain incomplete. Original inputs,
JAR/mappings/classpath/tools/raw/final digests are recorded. No original method
bodies or game binaries are distributed, and these facts do not relicense them.

`text_constructor_rules-1.21.11.json`, its compressed cases and source record
contain observed native fuzzy-mapper order, original constructor decode/getter
and rejection outputs, and original component comparisons. Owned tools call
unchanged original bootstrapping/codec/getter methods with pinned inputs;
bytecode inspection remains local. Factual candidate order does not complete
complex selector/profile/URI/dialog/item/entity constructors, persistent/cache
semantics or gameplay support. These facts contain no original method bodies or
game binaries and do not relicense Minecraft.

`profile_rules-1.21.11.json`, its compressed cases and source record contain
observed original profile NBT/stream constructors, name/model grammar, constructor
fields and native equality results. Owned standalone tools call unchanged original
codecs and getters without online profile/skin resolution. Original property
key iteration order and distinct constructor/encode/roundtrip outcomes remain
in the facts; comparison normalization is not persistent encoding or hash evidence.
Original JAR/mapping/classpath/tool/input/raw/final digests bind the facts.
Native method bodies and binaries remain local and are not distributed or
relicensed. Persistent/cache semantics and gameplay admission remain incomplete.

`selector_rules-1.21.11.json`, its compressed cases and source record contain
observed original selector option/type/character grammar, constructor cursors,
text/score fields, profile presence-byte outputs and original equality pairs.
Owned standalone tools call unchanged original bootstrapping/codecs/parsers/getters.
Compiled selector fields remain diagnostic facts, not live entity-query execution.
`character_names-21.0.12.1.json.gz` contains observed Character.getName/codePointOf
names and uppercase folds from the recorded JDK, bound to executable/modules hashes.
SNBT applies its own ASCII spelling gate after trimming; these facts are not JDK
method bodies or modules. Original game/JDK binaries, methods and inspection logs
remain local and are not distributed or relicensed. Full constructors, legacy
selector semantics, persistent/cache and gameplay admission remain incomplete.

`uri_rules-1.21.11.json`, its compressed cases and source record contain observed
original URI ASCII masks, non-ASCII exclusions, allowed schemes, native raw UTF-16
getter fields, nested text click routes and URI/component.equals results. Owned
standalone tools invoke unchanged original Minecraft codecs and JDK constructors.
Module opening enables read-only reflection, not method replacement. Original
JAR/mappings/classpath/JDK executable/modules/tools/input/raw/final hashes bind
the observations. Original methods, game/JDK binaries and inspection logs remain
local and are not distributed or relicensed. These facts do not prove URL opening,
DNS/HTTP access, legacy URI parity, persistent/cache semantics or gameplay admission.

`click_constructor_rules/cases/source` and `entity_tooltip_rules/cases/source`
record observed original chat character exclusions, builtin entity type names,
NBT stream acceptance, native constructor/getter fields and component.equals.
Owned standalone tools call unchanged original codecs and JDK UUID constructors.
Original JAR/mapping/classpath/JDK executable/modules/tool/input/raw/final hashes
bind these facts. Original method bodies, binaries and inspection logs remain
local and are not distributed or relicensed. These facts do not establish live
entity lookup, tooltip/UI execution, legacy event parity, persistent/cache hash
semantics or gameplay admission.

`fraction_constructor_rules/cases/source` record observed factory, arithmetic,
numerator/denominator and equals behavior of the unchanged Apache Commons Fraction
library in the original server bundle. The Rust bounded arithmetic is owned code;
original library binaries and method bodies are not distributed or relicensed.
`nested_item_constructor_rules/cases/source` record original nested item codec
acceptance, count/empty/property getters, original encode outputs, list lengths,
bundle Fraction/selected fields and native component.equals. Original prototype
inputs, JAR/mapping/classpath/JDK/tool/request/raw/final hashes bind these facts.
Whole Rust item/component comparison, live registry context, persistent/server
cache and gameplay admission remain separate obligations.

`effective_item_component_cases/source` record unchanged original ItemStack
stream decoding, count/empty getters, its effective component iterator and
one-time component stream encodings after prototype/patch application.
Original JAR/mapping/classpath/JDK/tool/request/raw/final hashes bind the facts.
Owned observers call original methods without replacement. Original binaries,
method bodies and runtime logs remain local and are not distributed or relicensed.
These field observations do not establish native whole-item equality, live
registry ownership, persistent/server cache semantics or slot admission.

`book_constructor_rules/cases/source` contain observed original writable/written
book and enchantability stream acceptance, raw/filtered getter fields, nested text
getters, original encode outputs and native value.equals. Owned tools call
unchanged original codecs and constructors. The observer's JSON transport escapes
UTF-16 units without changing native text values. Original JAR/mapping/classpath/
JDK/tool/request/raw/final hashes bind these observations. Original binaries,
method bodies and inspection logs remain local and are not distributed or
relicensed. These facts do not establish legacy book parity, lore derived styles,
live registry binding, full item/prototype comparison, persistent/server cache
semantics or gameplay admission.

`enchantment_constructor_rules/cases/source` record observed original stream
acceptance, effective reference/level map entries, original encode outputs and
native value.equals under the recorded vanilla registry context. Owned tools
call unchanged original codecs and getters; stream-map iteration order is not a
stable canonical byte identity. Original JAR/mapping/classpath/JDK/tool/request/
raw/final hashes bind these facts. Native binaries, method bodies and inspection
logs remain local and are not distributed or relicensed. This does not establish
live registry binding, complete item/prototype equality, persistent/server cache
semantics or gameplay admission.
