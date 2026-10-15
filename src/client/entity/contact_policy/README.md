# Contact policy shadow (migration step 2)

Production still reads `contact_capabilities`, `owned_contacts`, `geometry`, and
vehicle collision code. These private constants are compiled and validated on
normal builds, but only tests read their decisions. No public API or runtime
configuration is added. Wolf remains Unknown.

`table.rs` implements option B: named effect fields, normalized policy profiles,
explicit version/lane columns for every entity in the registry union, finite
packet profiles, locality requirements, semantics selectors, and evidence and
scenario descriptors. Entity assignments never inspect ancestry. Absent registry
pairs use `Absent`; present unaudited pairs use `Unknown`. Selectors that do not
apply use `NotApplicable`/`NoTickBody`/`None` explicitly.

The five lane columns repeat the **shared predicate observations** frozen in
step 1. They do not authorize the shared classification in every owner lane.
`LANE_CONTRACTS` keeps owned processing, received public geometry, and vehicle
sampling separate. `Profile::owned` names existing owned reducer/tick/shape/team
code; `semantics` returns the appropriate lane's selectors. Vehicle selectors
name branches in the existing sampler; dynamic receipts, ignored passengers,
locality checks, and proximity refusals remain its responsibility.

## Compile-time guarantees

`const _: () = validate_policy(...)` runs with Rust 1.85-compatible integer
indices, while loops, and assertions/panics. It checks:

- Complete, unique ordered entity and descriptor IDs; explicit presence/absence
  for both version columns and every named lane field.
- All seven named effect classifications and all 521 packet keys: nine packet
  families without sub-opcodes, all 256 signed Status values, and all 256 unsigned
  Animation values. Packet defaults must Refuse; exact rules must carry evidence.
- Packet rules expanded across their effect sets. Any overlap is rejected,
  including equal decisions, repeated effects, or different proof IDs. There is
  no override precedence or profile composition fallback.
- Descriptor references and selector combinations: modeled capabilities require
  owned numerical selectors, finite callbacks, and numerical scenarios; envelope
  profiles require all receipt/lifetime/peer/query obligations and appropriate
  evidence. Chicken/player/sheep/spider-specific selectors cannot be assigned to
  other species. Public geometry cannot select the private shape contract.
- Locality lane/effect/revocation uniqueness and owned-only applicability.

The Cartesian validation is factored, not a 5.8-million-cell allocation: each
packet profile expands into its complete effect x packet matrix, once. Each
explicit version x entity x lane assignment is then checked across all effects
and binds exactly one such matrix. Unique complete assignments multiplied by
unique complete matrices establish uniqueness and coverage of the full product.
The validator counts every represented packet cell, including explicit absent
pairs. This avoids repeating identical packet expansion work 1,590 times.

With the current registry there are 159 union entities, 264 present version/entity
pairs, 1,320 present lane assignments, and 270 absent lane assignments. The full
product represents 5,798,730 keys, of which 4,814,040 are present. The three packet
profiles expand 10,941 cells total (49 exact allows, 10,892 default refusals).
The nine entity profiles and two locality profiles are small constants; the
expanded packet booleans are retained only in tests (10,941 bytes).

## Test-time guarantees and limits

`baseline.rs` compares all 4,814,040 present keys directly with the existing
production capability and packet functions. A second rendering from the shadow
constants compares byte-for-byte with the unchanged step-1 golden, including
spawn dispatch, modeled status, and default-pose/far-query locality. The original
production rendering and representative typed refusal checks remain in place.

Additional tests check public geometry through the real `derived_geometry`
function for every present pair, rigid vehicle classification against the real
predicate, numerical species/scenario and exclusion membership in the existing
oracle, evidence artifact existence, and all 516 spider revocations for both
species/versions through the existing receipt reducer and owner tick. Validator
negative tests exercise missing rows/descriptors, duplicate keys, overlapping
packet expansions, and incompatible selectors. Registry JSON/name/ID/presence
agreement is still the bidirectional test in `key.rs`, not a compiler guarantee.

External artifact contents and hashes, original metadata field/serializer
resolution, dynamic selector behavior, numerical arithmetic, callback side-effect
closure, and arbitrary histories/queries are **not** proved by const validation.
Existing evidence-binding, locality, metadata, team, history, and numerical oracle
fixture tests remain independent checks. Scenario IDs identify existing proof
families, not a generated replacement for the fixed regression corpus. Static
metadata selectors refer to the existing reducer, with no copied numeric indices.

The development exporter is deferred: step 2 uses the in-memory golden rendering
for a complete semantic comparison. No public exporter, runtime manifest reader,
Python consumer migration, oracle regeneration, or policy digest is introduced.
