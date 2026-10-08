# 設置の事前確認（`Survival::placement_check`）

`client.survival().placement_check(support, face)` は、`support` の `face` に block を置く前に、受信済みの状態だけで設置の見込みを確かめます。packet は送りません。最終的な判断はサーバーです。

```rust
let check = client.survival().placement_check([x, y - 1, z], BlockFace::Up).await?;
if check.clear() {
    client.survival().use_on_block([x, y - 1, z], BlockFace::Up, [0.5, 1.0, 0.5], Hand::Main).await?;
}
```

## 返す値（`PlacementCheck`）

| field | 意味 |
| --- | --- |
| `support`, `target` | クリックする block と、置かれるセル（`support` を `face` 方向に 1 つ進めた位置） |
| `support_state`, `target_state` | 受信済みの block state（未ロードなら `None`） |
| `target_loaded` | `target` の列がロード済み |
| `face_distance` | 立った目の高さ（1.62）から、クリック面の最も近い点までの距離 |
| `interaction_range` | 公式クライアントの選択距離。1.21.11 は受信した `minecraft:block_interaction_range`（既定 4.5）、1.16.1 は 4.5（creative は 5.0） |
| `reachable` | `target_loaded` かつ `face_distance <= interaction_range` |
| `player_intersects_target_cell` | 自分の箱（0.6×1.8）が `target` セルと重なる |
| `blocking_entities` | `target` セルと既定の箱が重なる、建築を妨げる entity の native id |

`clear()` は「reachable で、自分も妨げる entity も target セルにいない」ことです。

## 建築を妨げる entity

公式の `EntityGetter.isUnobstructed` が見る `Entity.blocksBuilding` の型別の値に合わせています（両版の公式 jar で確認）。

- living entity（他の player、mob など）
- armor stand（marker でないもの。marker は client flags の 0x10、index 14 / 15）
- falling block、primed TNT、end crystal
- minecart と boat（1.21.11 の木材別 boat・raft を含む）

item、経験値オーブ、矢などは妨げません。

## 制限

- target は 1×1×1 のセルとして扱います。slab や松明など、置く block の実際の形はサーバーが決めます。
- target が置き換え可能か（空気・水・草など）は判断しません。`target_state` を見てください。
- 目の高さは立ち姿勢、entity の箱は型の既定寸法です（子ども・小型 armor stand の寸法は反映しません）。
- 1.16.1 のサーバーは足元から 8 block 以内の use-on を受け付けるため、`reachable` が偽でも置ける場合があります。1.21.11 は目から `interaction_range + 1.0` で拒否します。

## 公式サーバーでの確認

`examples/placement_check_probe.rs` で両版を確認しました。空き・自分の位置・豚（NoAI）・marker armor stand・通常の armor stand の 5 セルに石を置き、`clear()` とサーバーの設置結果がすべて一致しました。範囲外（目から 5.73）は `reachable == false` になりました。
