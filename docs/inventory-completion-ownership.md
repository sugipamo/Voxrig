# Inventory completion evidence ownership

The version packet reducer owns received slot and cursor values. Its current
inventory, strict click/transfer/take records, the executor comparison's before
snapshot, and crafting's operational owner may contain copies of the same cursor.
These copies answer different questions: latest received value, immutable strict
history, original intent basis, and operational craft proof. None is permission
to substitute a predicted cursor for a missing received one.

The defect was temporal. Reservation admitted untagged receipts; binding the
actual native action advanced `after_sequence` without discarding them. Separately,
a cursor from a preceding response could be newer than send but older than the
new full contents. Presence of three receipt flags then froze a divergence before
the matching cursor arrived. A terminal divergence intentionally does not mutate
when later packets arrive, so correcting that snapshot downstream is too late.

## Authoritative evidence

Only receipts after a complete original write, in the same connection, world and
opening, can discharge completion debt. The coherence gate (legacy) or state lock
(modern) excludes packet application while the send boundary and write are bound.
Reservation receipts still update the reducer but never become completion evidence.
Binding or refreshing the final boundary clears every completion candidate.

For the deliberately mismatching 1.16.1 click path the native response group is
`send boundary < matching action reply < full contents < cursor`. Full contents
never includes the legacy cursor. The server applies a click before comparing its
returned stack with the deliberately unequal stone x127 comparison. Its negative
comparison bit is retained in strict/native reply diagnostics; it supplies neither
a `Rejected` cause nor rollback proof. A refusal is detected from a complete
received boundary that retains the original slot and cursor values.

For 1.21.11 full contents includes the cursor atomically. Completion requires
`send boundary < contents == cursor`, plus a matching fence reply newer than the
send boundary. The fence and atomic full contents may arrive in either order.
An unrelated standalone cursor cannot replace the full packet's cursor evidence. Click, QUICK_MOVE,
SWAP/native execution, explicit resync and crafting share this executor rule.
Strict APIs still retain their original successor, interruption and deadline
contracts; partial crafting comparisons still diverge, and mo6 operational
crafting remains an independent owner.

## Retention and retirement

An incomplete group remains Pending and blocks subsequent writes. An exact ordinary
click/transfer comparison has an owned finite receipt deadline. On expiry it sends
one new no-effect PICKUP at slot -1 (never the original intent), then waits one
bounded resync interval. Both ordinary phases use `inventory_click_receipt_timeout`;
crafting retains its separate existing resync budget. The original send identity, action, before snapshot and
intention remain; the additional probe's action, fresh modern fence token and boundary are
recorded separately. Candidates are cleared again at that probe boundary, so the original
reply cannot discharge the probe's debt. Missing recovery receipts and uncertain
probe delivery end with typed inspection reasons. Late packets update observation
but cannot promote an expired operation to success. Explicit resync remains
available to obtain current state; it never replays the old click.

The executor owns only one current operation. A later admitted send supersedes
completion evidence and retained comparison/divergence according to the existing
API contract; callers must retain diagnostics before their next send. Scope
retirement invalidates debt rather than combining evidence across openings.
Strict and operational crafting histories keep their existing retention rules.

## Re-derivation cost

The normal path adds ordinal checks and waits for packets vanilla already sends:
no additional normal-path round trip. Clearing candidates is O(1); comparing a
complete boundary and retaining its before/current snapshot costs O(menu slots).
Only a missing group pays the first timeout, one O(menu slots) full resync and a
second bounded timeout. Original send identity/cause history cannot be reconstructed
from a later full inventory snapshot; retain it while the operation owns the
executor. Predictions and recipes are cheap to derive again from a fresh received
basis, but cannot reconstruct missing historical packets or justify a resend.
