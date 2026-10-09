# Java 1.16.1 Lighting Freshness

Lighting arrays are server observations, not client-side propagation. When a
cached block state changes light transfer or has uncertified optical behavior,
both light channels in that chunk and its eight horizontal neighbors become
unknown. A subsequent accepted Light Update
restores only the channels/sections it supplies. Duplicate block-state updates
do not erase current observations. Old immutable snapshots remain historical.

Changing between certified opaque, non-emitting full cubes retains the received
arrays, including surrounding chunks and other heights. Certification uses the
pinned native registry (emission 0, filtering 15, nontransparent) and full-cube
collision shapes. It covers single-state blocks and explicitly checked snowy
grass/podzol and log/wood axis variants. Type-level registry defaults never
certify arbitrary lit, waterlogged, charged or partial-shape variants. Unknown
states remain conservative. This applies to block changes, multi-block changes,
acknowledged states and full/partial chunk redelivery. Omitted terrain sections
in full chunk data are air; partial delivery preserves them.

Retaining a previously received value is not a new server light receipt. An
equivalent change never restores arrays already invalidated by an earlier
light-affecting change. Source addition/removal, transparency changes and
uncertified state transitions still require new light evidence.

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
