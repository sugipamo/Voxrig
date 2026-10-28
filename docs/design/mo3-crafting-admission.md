# MO3 A: held at the strict API compatibility decision

Baseline: origin/main 199e7ba1. This PR deliberately contains a regression
characterization, not a production fix. Do not merge before choosing the API
contract below.

## Ownership audit

1. The server owns grid/result, cursor, main/hotbar and transaction replies.
   `CraftingTakeRecord` owns the conjunction of those receipts for one take;
   the independent inventory executor owns response/state progress. Dispatch,
   prediction and an executor terminal status are not crafting completion.
   `craft_once` owns a helper result and explicitly returns `Ok` for
   `RequiresInspection`; `craft_once_checked` requires `ObservedTransferred`.
   The mismatch can therefore be in a consumer interpreting `Ok` as confirmed
   completion. There is no minegpu packet trace in this checkout to identify the
   exact missing receipt in its one failing run. Claiming a proven packet race
   or a pending record returned by current `craft_once` would be unsupported:
   current `craft_once` returns Timeout for Pending.
2. Only fresh received values are authoritative. A complete post-send boundary
   must include grid/result, cursor, main/hotbar, and the legacy transaction reply.
   A mismatch reply is not rollback; a sent resync is not a receipt.
3. Pending currently has no record-owned expiry. Inspection is latched. Neither
   timeout nor permitting independent motion can discard an uncertain inventory
   write. Retain its identity and evidence until explicit reconciliation or a
   retired session/opening; never replay the result click.
4. Reading a retained record is bounded local work; rechecking the native grid
   and inventory is O(grid + 36). Full resync costs one no-effect probe and one
   server round trip/full-content response. A future two-phase deadline should
   default to 5 s per phase, matching inventory_click_receipt_timeout, configurable
   through common limits on both adapters. This is a proposed budget, not a
   measured latency guarantee.

## Reproduction and incompatible choices

The shared packet-adapter crafting matrix now withholds all post-send receipts
and performs 64 strict Look attempts. Each is refused; the retained take stays
Pending. The matrix then supplies actual values and the original response, so
it also checks that successful completion still requires received evidence.

`docs/domain-gates.md` explicitly preserves all-family strict admission for
finite motion, and routes independent Look/Move through HeadlessBody already.
Changing `common_motion_admission` globally would also weaken inventory swap,
recipe placement, placement and mining callers. The modern strict Look similarly
uses `mutable`, while Body Look explicitly selects Domain::Motion.

Choose one before implementation:

- Keep strict compatibility: migrate Golemkit Look/Move to HeadlessBody and craft
  completion to craft_once_checked. Add a take-owned two-phase timer, a no-effect
  full-resync request (never replay the take), and a typed terminal inspection
  reason separately. Keep strict inventory/screen vetoes.
- Change existing strict Look/finite motion admission: split actual motion call
  sites from the shared all-family helpers in both adapters, retain strict
  mutation admission, and document the behavior break. Decide whether craft_once
  should also reject RequiresInspection rather than silently changing it here.

Golemkit can already classify the non-exhaustive stage: Pending and
RequiresInspection are unresolved; ObservedTaken, ObservedTransferred and
NotApplied are resolved. The record reader is currently on Survival/Creative,
not Client. A public record/stage is_unresolved helper would reduce downstream
match drift; it is proposed with the production fix, not added by this test PR.

The missing-boundary reproduction does not establish the frequency or the exact
wooden_pickaxe table failure reported by minegpu. LIVE healthy crafting is a
separate integration check; deterministic missing receipts are packet-fixture
coverage, not an unmodified-server observation.

LIVE 1.16.1 healthy PICKUP and craft_once QUICK_MOVE both completed with fresh
receipts and independent RCON inventory checks. Evidence:
`docs/evidence/mo3-crafting-baseline-live-1.16.1-20261028.json.gz`.
This healthy run does not reproduce minegpu's intermittent table failure.
