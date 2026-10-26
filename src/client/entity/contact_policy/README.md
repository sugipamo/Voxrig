# Contact policy (migration step 4b)

Production `contact_capabilities` remains the compatibility facade and reads
capability classifications and harmless Status/Animation decisions from this
const table. The shared predicates use the OwnedWalking observation column;
this does not add lane admission. Unresolved/version-absent names return all
Unknown/false. Spawn/tick dispatch, owned shape, metadata handler, team holder,
UUID/profile identity selection and public geometry now read semantics selectors
through `selectors.rs`. Locality eligibility, envelope selection, revocation sets
and owned usage gates now read `locality.rs`. Dynamic proofs and vehicle sampling
retain their existing implementations. No public API or runtime configuration
is added. Wolf remains Unknown.

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

`const VALIDATED: () = validate_policy(...)`, referenced by lookup, runs with Rust 1.85-compatible integer
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

`baseline.rs` compares all 4,814,040 present keys with production and the frozen
pre-step-3 capability match and harmless-packet allowlist. The match is moved
byte-for-byte to test-only `frozen_capabilities.rs`; its historical evidence hash
is unchanged. Both the production and table renderings still compare byte-for-byte
with the unchanged step-1 golden, including spawn dispatch, modeled status, and
default-pose/far-query locality. Separate tests compare unresolved and absent names
against the frozen source for every Status/Animation opcode. Representative typed
refusal checks remain in place.

`frozen_selectors.rs` retains the pre-step-4a Spawn and public geometry branches,
the shared metadata/team/UUID selections and the player-profile identity branch.
Its modeled boundary reads only the independent frozen capability match. For all
264 present version/species pairs (and absent pairs), tests compare the table's
owned selectors in each owned lane against that source. The production adapters
are checked against those old choices, including metadata invocation/error
propagation, team membership/missing holder, UUID/profile selection and bitwise
shape results for every native pose and representative baby/scale factors. The
baseline also compares actual reducer Spawn dispatch and actual public geometry
with the frozen source. Expected choices never come from table assignments.

The owned adapters observe the same OwnedWalking column as the shared capability
facade; they run after the existing admission gates. They do not grant eligibility
in another lane. Enum dispatch is exhaustive. Nonapplicable owned handlers cannot
be called for an admitted body under the const-validated selector/capability
invariants. The metadata wrapper selects the existing living reducer; its field
resolution, arithmetic, state updates and refusal construction remain there.
IdentityModel::UuidHolder still requires the received UUID; TickModel::RemotePlayer
selects the original registered-profile/name/game-mode path. TeamModel::OwnHolder
calls the original team lookup. Chicken state and arithmetic in contact_tick.rs
are untouched. Public geometry still supports only player/cow/pig/zombie; the
vehicle sampler does not read these adapters.

Additional tests check public geometry through the real `derived_geometry`
function for every present pair, rigid vehicle classification against the real
predicate, numerical species/scenario and exclusion membership in the existing
oracle, evidence artifact existence, and all 516 spider revocations for both
species/versions through the existing receipt reducer and owner tick. That corpus
is now enumerated from the frozen old packet-family match, so removing a table
key cannot remove a regression case. Validator negative tests exercise missing rows/descriptors, duplicate keys, overlapping
packet expansions, and incompatible selectors. Registry JSON/name/ID/presence
agreement is still the bidirectional test in `key.rs`, not a compiler guarantee.

External artifact contents and hashes, original metadata field/serializer
resolution, dynamic selector behavior, numerical arithmetic, callback side-effect
closure, and arbitrary histories/queries are **not** proved by const validation.
Existing evidence-binding, locality, metadata, team, history, and numerical oracle
fixture tests remain independent checks. Scenario IDs identify existing proof
families, not a generated replacement for the fixed regression corpus. Static
metadata selectors refer to the existing reducer, with no copied numeric indices.

## Audit connection

`python3 -B scripts/audit_contact_capabilities.py --check` runs the private
`audit::export_contact_capability_policy` Rust test with `-j1` and
`--test-threads=1`, reads its marked JSON stdout, and compares all seven evidence
classifications and the complete version/entity sets in both directions. The
ordinary Rust suite also performs this evidence/table comparison directly.
This is development-only export, with no public exporter or runtime config.

The Python auditor no longer owns qualification lists or emits Rust. When run
with original `--downloads`/`--work` inputs, it takes required classifications
and reviewed-type fingerprint inputs from the Rust export, checks original-client
entity coverage, and retains the existing independent method-owner/fingerprint
and review/blocker evidence collection. Ordinary-mob review inputs are derived against the historical pre-extension
evidence; blocker prose remains an independent qualification fence. Its `--output` flag also selects the evidence for
`--check`; check mode writes nothing.

Schema-2 evidence field names are retained: `generator_sha256` binds the auditor
script, while `generated_sha256` now binds the frozen historical match (same
bytes/hash). The whole-source envelope binding of `contact_capabilities.rs` is
updated in step 3 because the facade delegates to the table and the evidence test
includes the moved historical file. Step 4a updates only the envelope binding for
`owned_contacts.rs`: the inline selectors must call the thin private facade, and
metadata gains a dispatch wrapper around the existing reducer. Its admission,
locality, revocation and usage gates are unchanged. The independent frozen-source,
golden and regression comparisons check the bound decisions. No oracle artifacts,
golden, original-client method hashes, or other source bindings change in 4a.
The audit reviews now document the current export/check paths.

Step 5 completes the shared export and consumer checks described below. Existing fixtures and reviews remain
independent evidence; table output is never a numerical oracle.

## Step 4b locality and usage reads

`locality.rs` selects the existing DefaultSpiderEnvelope handler, applicable
owned lanes/effects and revocation set from the const profiles. DeferredContact
retains a small static policy value selected at construction with the same
version/name as its capabilities. Its walking/boat relevance uses the corresponding
lane's solid plus walking/incoming effect fields. Spawn admission reads modeled
effects in the existing walking/glide/boat lane. The shared lane observations are
unchanged; none grant public geometry or vehicle sampler admission.

The original pose/dimension checks, packet provenance, lifetime matching,
passenger membership, monotonic envelope arithmetic, interpolation endpoints,
peer/query intersection and receipt invalidation remain in their original code.
Invalid Motion provenance/codec application still invalidates the proof directly;
it is a dynamic failure, separate from the static revoking packet set. No profile
requirement is optional and no new proof domain is introduced.

`frozen_locality.rs` preserves the old spider species expression, walking/boat
relevance, modeled gate (using the independent frozen capability source), and
revoking packet families. Tests compare table/production/old choices for every
present and absent version/species pair and unresolved names, every packet key,
walking/boat usage, glide admission, absent/present poses, and near/far queries.
All 2,064 old spider revocation cases still execute through the real reducer and
tick. Existing checkpoint, motion, lifetime, peer/query, passenger, boat and sweep
regressions retain their independent expectations. The step-1 golden is untouched.

Step 4b updates only the envelope evidence source hashes for
`contact_capabilities.rs` and `owned_contacts.rs`: the former routes static choices
through the facade and separates static revocation from dynamic Motion failure;
the latter routes Spawn's modeled gate through the lane lookup. The old
Capabilities::modeled predicate is now test-only, alongside its frozen consumers.
Numerical oracle
fixtures, original-client hashes and all other evidence values remain unchanged.


## Step 5 consumers and docs

`audit::export_contact_capability_policy` now emits `CONTACT_POLICY=` JSON,
validated by `scripts/contact_policy_export.py`. Schema 1 exports the resolved
lane/effect profiles, actual lane selectors, packet defaults/exact rules,
locality eligibility/requirements/all 516 revocations, scenario/evidence IDs,
vehicle selectors, native IDs and versioned classifications. The present-key
summary includes all axis values and the 4,814,040-key count. `policy_digest` is
SHA-256 of compact sorted JSON before adding that field, independently checked
in Python. Use the wrapper without flags for clean JSON stdout.

The capability auditor, packet auditor and envelope auditor read that export;
each has a jar-independent `--check`. Manual opcode proofs, source fingerprints,
blocker judgments and original numerical values remain independent evidence.
The owner consumer checker compares required species/scenarios, both scenes,
fixed (scene, seed) IDs, original Java factory constructors/registration output,
and exclusions in both directions. It also accepts a temporary original rerun
for exact scenario/value/raw-output-hash comparison. The original Java harness
and runner remain byte-identical and never consume generated numerical logic.

Six marked documentation sections are rendered by the export wrapper;
`--check` compares in memory and writes nothing. CI runs all five checks.
See [consumer commands](../../../../docs/contact-policy-consumers.md).
The fixed admission target set now checks equality with the table rather than
only membership. Fixed raw-bit numerical corpora, fixed stage counts, historical
frozen sources, golden bytes, and refusal regressions remain unchanged.
