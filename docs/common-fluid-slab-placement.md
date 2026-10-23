# 液体を明示したoak slab設置（1.21.11）

`Survival::place_slab_with_fluid(clicked, face, target, before)`は、選択中のdefaultな
`minecraft:oak_slab`を1個だけ使う追加入口。`fluid_slab_placement_record()`で
`FluidSlabPlacementRecord`を読む。既存の[`place_slab`](common-slab-placement.md)はdry限定のまま。
1.16.1では新操作・新getterの両方が明示Unsupportedになる。

```rust,no_run
use voxrig::client::prelude::*;
# async fn place(client: &Client) -> Result<()> {
let before = voxrig::NativeBlockState {
    name: "minecraft:water".into(),
    properties: [("level".into(), "0".into())].into(),
};
let attempt = client.survival().place_slab_with_fluid(
    [3, 66, 0], BlockFace::West, [2, 66, 0], before,
).await?;
let progress = client.survival().fluid_slab_placement_record().await?;
# let _ = (attempt, progress);
# Ok(())
# }
```

## 対象と原版fluid state

健康なsurvival・通常立位・接地・通常属性・空cursor・受信済みの選択材料とplayer screen基準を
要求する。位置は正確な静止受信起点に限定する。最初のnative outline、面、4.5block以内の
交点とcursorを、player・block・材料と同じlock境界で決める。fluidそのものを照準のoutlineにしない。
クリック対象は既存のpassive cubeまたは完全なoak slab state。原版の同一cell二重化／面の隣接cellの
選択と、明示した`target`・完全な`before`が一致しなければ送信しない。

対象はpropertiesが空のair、全16種類の`water[level=0..15]`、または同じoak slabのtop/bottom。
slabには`type`と`waterlogged=true|false`の両propertyが必要。異種slab、既存doubleの置換、
溶岩、bubble column、kelp、waterlogged stairs等の未監査family、未知fluidは明示拒否する。
名前・fluid tag・amountだけで置換可否やwaterloggedを一般化しない。

| 明示した対象の元state | native fluid | 生成slabと液体 |
| --- | --- | --- |
| air | empty | top/bottom、`waterlogged=false` |
| water level 0 | source WATER、amount 8 | top/bottom、`waterlogged=true`、sourceを保持 |
| water level 1..7 | FLOWING_WATER、amount 7..1 | top/bottom、`waterlogged=false`、対象の流水を置換 |
| water level 8..15 | falling FLOWING_WATER、amount 8 | top/bottom、`waterlogged=false`、対象の流水を置換 |
| dry oak top/bottom | empty | double、`waterlogged=false` |
| wet oak top/bottom | source WATER | double、`waterlogged=false`、対象の水を置換 |

原版`SlabBlock.getStateForPlacement`は新しいhalfではfluidの**型がsource WATERと同じか**を
判定する。amountが8のfalling waterも別型である。同種slabの二重化は先に別分岐へ入り、
元のwaterloggedをfalseへ戻す。従って「targetがwaterなら常にwaterlogged」ではない。
half選択の面・hit高さ0.5の境界はdry入口と同じ原版規則を使う。

設置専用の立位haloだけで、監査済みの水／wet oak slabを許す。playerの身体がfluid cellに
重なれば拒否する保守的な条件で、泳ぎや水中操作へは広げない。own bodyと受信entityに対する
実際のtop/bottom/double collision判定はdry入口と共用する。他の立位・移動consumerは広げない。

## 元状態の由来と独立した受信

`FluidSlabPlacementRecord.placement`は、元の完全な支持／対象block、first outline、材料と
受信ordinal、intent、完全送信、独立した対象・材料・ACK受信を持つ。
`source`は両cellを読んだraw受信world cacheのversion、接続、capture時点の受信境界、revisionと範囲。
`source.region`には元のクリックcellと対象cellを含む最小範囲を記録する。
個別block packetの受信ordinalを捏造しない。piston等のlocal reconstructionに由来する
支持／対象はこの入口で拒否する。`support_fluid`・`before_fluid`・`expected_fluid`はそれぞれの
完全blockから原版規則で導いたemptyまたはsource/falling/amountで、独立したfluid packetではない。

完了には完全送信、freshな完全対象state、ちょうど1個の材料消費、送信境界より新しいmodern ACKが
すべて必要。最後の1個はfreshなEmptyを要求する。ACKだけ・cache一致・隣接水更新は成功証拠にしない。
fluid更新の安定や周辺流れの収束を`ObservedPlaced`の条件へ混ぜない。
設置前の流水が別levelへ変われば、完了前の対象競合として保持する。

既存placement ledgerの排他・connection/world/pose/selection/screen寿命を使う。
取消・部分write・途中chunk unload/再置換・block/material競合で再送やrollbackをしない。
完了したreceiptと元fluidは、その後のblock・流体・在庫・ACKで書き換えず、次の別操作や接続終了後にも
保持する。dry slab、液体対応slab、cube、植物置換、stairsの履歴getterは別々に最後の対応操作を返す。
別actorの操作との因果帰属は保証しない。

## 根拠と再実行

[原版driver](../scripts/ExportFluidSlabPlacement.java)は未改変の`BlockPlaceContext`、
`SlabBlock.canBeReplaced/getStateForPlacement`、blockからの`FluidState`とその型・source・falling・amountを
呼ぶ。cell/fluid参照とsecondary-use入力だけを供給し、予期しないworld呼出しを拒否する。
[source record](../data/client_api/fluid_slab_placement_source.json)に公式jar・mappings・classpath・
generator・fixtureのhashを保持する。5,328件は6面、0.5の直前／同値／直後、secondary-use両値、
source／全flow level、wet/dry oak slab、異種slab、溶岩・bubble column・kelpを含む。
既存のslab outlineとcollisionの原版fixtureも引き続き検査する。

構成packet試験はsource／flow／falling／dry／wet slab、二重化、全6受信順、最後の1個と複数個stack、
各受信の欠測・古いACK、途中fluid level／waterlogged／支持／選択／材料競合、復帰、
隣接水更新、途中UNLOAD_CHUNK、writer前取消・部分write、一度性、別入口・接続終了後の履歴を検査する。

```bash
python3 -B scripts/export_fluid_slab_placement.py \
  --jar /path/to/1.21.11/server.jar --mappings /path/to/server-mappings.txt \
  --modern-classpath-file /path/to/classpath.txt --work /tmp/fluid-slab-oracle --check
python3 -B scripts/run_fluid_slab_placement_live.py --accept-eula \
  --server-jar /path/to/1.21.11/server.jar \
  --binary target/debug/examples/climbing_control_probe --output /tmp/fluid-slab-live
```

live runnerは一時directoryの公式1.21.11だけを通常tickで使う。乾いた高所の足場と、
原版の水源で維持する水路から、安定した水平／fallingの流水predecessorを受信する。
操作の実受信・RCON照合を完了した後、周辺の流れを別に観測する。fluid競合が起きないという保証ではない。

2026-10-23の公式1.21.11実接続は10件を通過した。source水のtop/bottom、水平流水level 1/7、
falling level 8、dry cell、wet top/bottomの同一cell double化、隣接wet halfのdouble化、
dry bottomのdouble化を照合した。各操作は1個の材料・1回の設置packetで、完全な対象state、
freshなEmpty、modern ACKと独立RCONが一致した。流水の供給源がsourceのまま残ることもRCONで
確認し、source保持slabからの周辺流水の発生は、完了後の別観測として記録した。
[source・binary・元packet/RCONのhashと受信結果](evidence/fluid-slab-placement-20261023.json)を保持する。
probe・proxy・serverは終了し、元のserver directoryは変更していない。
