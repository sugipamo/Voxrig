# 共通のblock検索とraycast

1.16.1・1.21.11の両方で使える。どちらも1回の観測（`capture`）の中で計算するので、
途中で状態が入れ替わることはない。結果はclientが受信した状態であり、サーバー上の確定ではない。

## `find_blocks(region, names)`

`region`の中で、名前（例: `"minecraft:oak_log"`）が`names`に含まれるセルを返す。

- `matches`: 一致したセル（観測順）。
- `unloaded`: 未ロードのセル数。0でなければ検索は不完全。未ロードをairとして扱わない。
- `region`の上限は観測と同じ262,144セル。

## `raycast_blocks(origin, direction, max_distance)`

`origin`から`direction`方向へ最大32ブロック、blockの**衝突形状**に対して光線を飛ばす。

| 結果 | 意味 |
| --- | --- |
| `Hit { position, state, face, point, distance }` | 衝突箱に入った。`face`は入った面 |
| `Miss` | 通過したセルはすべてロード済みで、何にも当たらなかった |
| `Unloaded { position }` | 当たる前に未ロードのセルに達した |

- 光線が通るセルを順にたどり（Amanatides–Woo）、最初に当たったセルで止まる（vanillaのclipと同じ順序）。
- 衝突形状は各版のデータを使う（1.16.1は`data/block_collision_shapes.json`、1.21.11は
  `data/java_1_21_11/collision_shapes.json`）。草花のように衝突形状がないblockは通り抜ける。
  照準用の外形（outline）での判定は`survival().target_block`を使う。
- 世界の高さの外側のセルは空として扱う（実在しないため）。
- 始点が衝突箱の内側にある場合、その箱は当たりとして数えない。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、平坦な世界で`examples/entity_block_probe.rs`を実行した。

| 版 | 下向き（目の高さから） | 上向き | 周囲9×5×9の`grass_block`検索 |
| --- | --- | --- | --- |
| 1.16.1 | 足元の`grass_block`の`Up`面、距離1.62 | `Miss` | 81件、未ロード0 |
| 1.21.11 | 足元の`grass_block`の`Up`面、距離1.62 | `Miss` | 81件、未ロード0 |

1.16.1は`wait_until_ready`の直後には周囲の地形がまだ届いておらず、検索・raycastが
未ロードを返した。地形を使う前に`wait_for_loaded`で待つ。
