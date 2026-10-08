# 共通のchunk列（block stateと光）

`Client::loaded_chunks()`は現在のworldで読み込まれているchunk列の座標を、
`Client::chunk([x, z])`は1列分の受信したblock stateと光（`ChunkObservation`）を返す。両版で同じ型。

```rust,no_run
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let loaded = client.loaded_chunks().await?;
for &position in &loaded.chunks {
    let Some(column) = client.chunk(position).await? else { continue };
    let p = [position[0] * 16, 64, position[1] * 16];
    let _id = column.state_id(p);          // 版に結び付いたnativeのblock state ID
    let _state = column.block(p);          // 名前と全property
    let _light = (column.sky_light(p), column.block_light(p)); // 受信した光。None は不明
}
# Ok(())
# }
```

- `state_id`・`state`・`block`は、その列の中の座標なら必ず値を返す（列に含まれない座標や高さの外は`None`）。
  1.16.1で送られていないsectionは空気。
- section（16×16×16）はadapterのcacheと共有し、写さない。変更があったsectionだけが新しく作られる。
  両版の公式serverで、読み込まれている約120列（1.21.11で約1150万cell）をすべて読むのに1回あたり2〜4 msだった。
- 範囲を指定する`observe_region`（26万cellまで）・`find_blocks`・`raycast_blocks`はこれまでどおり使える。

## 光

光は**serverから受信した値**だけで、Voxrigは光を計算しない（公式clientは自分で伝播させる）。

- 1列の光は、高さの下に1つ、上に1つを加えたsectionごとに持つ。受信していないsectionは`None`（不明）。0とは読まない。
- 1.21.11はchunkのpacketに光が入っている。光の更新packetは、読み込まれている列にだけ適用する。
- block stateが**実際に変わった**受信（同じ状態の再送は除く）で、その列と周囲8列の光はすべて不明になる
  （1.16.1の[光の鮮度](lighting-freshness.md)と同じ規則）。
- 公式serverは、近くのclientには光の更新を送らないことが多い。
  2026-10-08に両版の公式serverで、playerの2つ隣に松明を置いた。block stateの変化は受信したが光の更新は来ず、
  その列と隣の列の光は不明（`None`）のままだった。
  変化の後の光を知るには、chunkの再送（読み込み直し）か再接続が必要。
