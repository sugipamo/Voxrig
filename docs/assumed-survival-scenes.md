# Read-only assumed survival scenes

`checked_survival::AssumedSurvivalScene::new(region, blocks, start)` admits
caller-supplied geometry for the same Java 1.21.11 dry-cube model as captured
scenes. It performs no network I/O and never constructs `StandingContext`.

The complete map must contain exactly every cell in the region: ordered axes
of at most 64 cells, at most 32768 cells total and bounded native coordinates.
Every state passes native state validation and existing dry-cube/air admission.
Missing outside cells remain unknown. Unknown or dynamic blocks refuse input.

`AssumedSurvivalStart` declares dimension, finite feet, bounded initial velocity
phase and per-axis geometric reserves. Defaults of the existing grounded model
are assumed: native normal movement speed, gravity, jump and step height, with
no effects, sprint or sneak. Standing support and body clearance must pass the
same native geometry check. Velocity components are bounded to [-1, 1]; reserve
components to [0, 1]. A zero velocity means reset initialization, not necessarily
the retained gravity phase of a resting model. No attributes are received.

Both entry points create `SurvivalScenario` with shared private geometry and the
same movement, ray, reach, standing, placement and removal checks. Branch edits
retain copy-on-write geometry and branch identity. Native edit/tick limits are
unchanged. Callers still select controls, routes, resource budgets and tasks.

`HypotheticalMovementPreview::source` is now `HypotheticalSceneSource`, tagged
as `Captured { standing }` or `Assumed { start }`. `source.captured()` is `None`
for assumptions, including after motion, edits and prospective reconnects.
`AssumedPosition` cannot validate actual standing evidence. Subsequent endpoint
and reconnect requirements describe future obligations only; they do not convert
assumed provenance into capture authority.

`Operations::validate_survival_scene` still accepts only `CapturedSurvivalScene`.
Actual motion still requires the separate native movement preview and operation
admission. Diagnostic records have one-way conversion from checked values and
cannot deserialize native intents, scenes or previews. Historical movement
records now carry tagged source provenance; old direct-standing source records
need explicit data migration if callers retain them. No reverse conversion is
provided.

On the common Client branch, component-bearing `InventorySlot` values project to
`diagnostic::RecordedInventorySlot`. Component patches and validated registry IDs
likewise have separate `Recorded*` facts. Persist those records rather than
deserializing native slots or IDs; wire bytes, patch removals and version ownership
are retained, while private slot/cursor/screen guards remain live-only.

This API supports reproducible model checks, not an independent Minecraft
physics oracle, live freshness proof, inventory receipt or executable plan.

Validation is offline. The public integration tests exercise complete geometry,
invalid/nonfinite starts, unsupported blocks, unknown outside bounds, native
placement, immutable branches and retained assumed provenance. A native fixture
compares captured and assumed motion frames with the same geometry/start phase
and refuses assumed aim evidence as live standing. Compile-fail doctests check
that assumed scenes cannot enter live scene validation, and hypothetical previews
cannot enter actual motion execution. These tests do not contact Minecraft.
