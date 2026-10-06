# Java 1.16.1 Lighting Freshness

Lighting arrays are server observations, not client-side propagation. When a
cached block state genuinely changes, both light channels in that chunk and its
eight horizontal neighbors become unknown. A subsequent accepted Light Update
restores only the channels/sections it supplies. Duplicate block-state updates
do not erase current observations. Old immutable snapshots remain historical.

An initial Light Update may precede the first terrain packet: those arrays are
retained. An identical terrain redelivery also retains them. Changed replacement
terrain invalidates lighting; partial terrain delivery preserves omitted sections.
Unloading a chunk discards its terrain and lighting together.

Vanilla 1.16.1 can omit a light update following torch placement. Clients must not
treat the placement-time arrays as new evidence, predict propagation, or turn
missing light into zero. Re-observation can use server chunk unload/redelivery,
or an explicit controlled reconnect. Changing a client's interest region alone
is not proof that the server unloaded a chunk. This cache policy does not prove
nighttime safety, parcel coverage, enemy absence, or server quiescence.
