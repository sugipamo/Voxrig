# 確認付きoak slab設置（1.21.11）

`Survival::place_slab(clicked, face, target, before)`は、選択中のdefaultな
`minecraft:oak_slab`を1個だけ使う追加入口。`slab_placement_record()`で専用の
`SlabPlacementRecord`を読む。1.16.1では両方が明示Unsupportedになる。

```rust,no_run
use voxrig::client::prelude::*;
# async fn place(client: &Client) -> Result<()> {
let before = voxrig::NativeBlockState {
    name: "minecraft:air".into(), properties: Default::default(),
};
// 現在のfirst outline・面と、利用側が選んだ実際の対象・完全なpredecessor。
let attempt = client.survival().place_slab(
    [3, 66, 0], BlockFace::West, [2, 66, 0], before,
).await?;
let progress = client.survival().slab_placement_record().await?;
# let _ = (attempt, progress);
# Ok(())
# }
```

## 対象とnative規則

既存の確認付き設置と同じ、健康なsurvival・通常立位・接地・通常属性・乾いた既知geometry、
実受信のown pose・空cursor・player screen基準を要求する。材料は実受信のdefault stackだけ。
今回は正確な静止位置に限定し、移動後の位置誤差が残る起点を拒否する。
クリック対象は既存のpassive cubeまたは完全な乾いたoak slab state。
4.5block以内のfirst native outlineと面が一致することを同じlock境界で検査し、
hitから得たcursorを送る。利用側がhit高さや期待halfを捏造する引数はない。

| 条件 | 選択 |
| --- | --- |
| bottom slabの上面、または側面のhit高さが0.5より上 | クリックcellをdouble化 |
| top slabの下面、または側面のhit高さが0.5以下 | クリックcellをdouble化 |
| その他の許可されたクリック | 面の隣接cell |
| 選択した対象が同種の乾いたtop/bottom slab | double、`waterlogged=false` |
| 選択した対象がairで下面クリック | top、`waterlogged=false` |
| 選択した対象がairで側面hit高さが0.5より上 | top、`waterlogged=false` |
| その他のair設置 | bottom、`waterlogged=false` |

実際の対象が明示した`target`・完全な`before`に一致しなければ送信しない。
異種slab・既存double・水中／waterlogged・植物等の置換には広げない。
同じcellのdouble化を、隣接airへの設置として扱わない。

設置後のtop/bottom/doubleのcollision shapeとown bodyを比較する。
受信entityもbuildingを妨げる型について同じshapeと比較する。livingは既存の資格付き
player/cow/pig/zombie geometryからpose・baby・実受信scaleを使い、位置量子化誤差を拡張する。
固定形状のTNT・falling block・end crystalも検査する。未知型・資格不足・その他の
building entityの形状は拒否する。全entityの姿勢や補間を既定boxへ置き換えない。
これは同じ受信境界での事前確認であり、未受信entityやその後のserver側変化は保証しない。

## 保持と結果

専用recordにはfirst outline、`merges_clicked`、実際のtarget、完全なbeforeとexpected、
元のplayer/material、送信前intentと完全送信を保持する。`expected`には必ず
`type`と`waterlogged`の両propertyを含める。
`PlacementStage`の意味は[共通設置](common-survival-placement.md)と同じ。
完全送信、新しい対象block受信、ちょうど1個の材料消費、送信境界より新しいmodern ACKが
すべて揃って初めて`ObservedPlaced`になる。最後の1個にはfreshなEmptyが必要。
cache一致・ACKだけでは完了しない。別actorとの因果を識別する保証はない。

既存placement ledgerの一度限りの排他を使い、未解決中の視点・選択・別操作を拒否する。
取消・部分write・world/screen/pose/selection変更・chunk置換・途中のblock/material競合は
保持し、自動再送やrollbackをしない。クリックcell自身のexpected double受信は支持喪失にしない。
完了したreceiptはその後の現在状態で書き換えず、次の操作や接続終了後にも読める。
旧cube・植物置換・slabの各getterは最後の対応する操作を別々に保持する。

## 根拠と再実行

[原版driver](../scripts/ExportSlabPlacement.java)は、未改変の`BlockPlaceContext`と
`SlabBlock.canBeReplaced/getStateForPlacement`を呼ぶ。cell/fluid参照とsecondary-use入力だけを
供給し、予期しないworld呼出しは失敗させる。
[source record](../data/client_api/slab_placement_source.json)には公式jar・mappings・driver・
classpath・fixtureのhashを保持する。2,520件のcontextは6面、0.5の直前／同値／直後、
同種・異種・double・水を含む。360件の原版clipと216件のnative shape交差を別に照合する。
fixtureはserverの設置権限やentity dimensionの全体を証明するものではない。
構成packet試験はblock・材料・ACKの独立した受信順と欠測、完全state競合、元状態への復帰、
chunk変更、身体／entityのhalf交差、取消した未送信intent、履歴と一度性を検査する。

```bash
python3 -B scripts/export_slab_placement.py \
  --jar /path/to/1.21.11/server.jar --mappings /path/to/server-mappings.txt \
  --modern-classpath-file /path/to/classpath.txt --work /tmp/slab-oracle --check
python3 -B scripts/run_slab_placement_live.py --accept-eula \
  --server-jar /path/to/1.21.11/server.jar \
  --binary target/debug/examples/climbing_control_probe --output /tmp/slab-live
```

2026-10-23の公式1.21.11実接続は、側面からのtop/bottom、bottomの上面とtopの下面からの
同一cell double化、隣接cellのbottomのdouble化の5件を通過した。
各回1個だけのoak slabを消費し、完全property state、freshなEmpty、modern ACKと
独立RCONを照合した。誤ったpredecessorは無送信、正しい操作の設置packetは各1回だった。
[source・binary・元packet/RCONのhashと受信結果](evidence/checked-slab-placement-20261023.json)を保持する。
probe・proxy・serverはすべて終了し、元のserver directoryは変更していない。
