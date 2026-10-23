# 確認付きoak stairs設置（1.21.11）

`Survival::place_stair(clicked, face, target, before)`は、選択中のdefaultな
`minecraft:oak_stairs`を1個だけ使う追加入口。`stair_placement_record()`で専用の
`StairPlacementRecord`を読む。1.16.1では両方が明示Unsupportedになる。

```rust,no_run
use voxrig::client::prelude::*;
# async fn place(client: &Client) -> Result<()> {
let before = voxrig::NativeBlockState {
    name: "minecraft:air".into(), properties: Default::default(),
};
let attempt = client.survival().place_stair(
    [3, 66, 0], BlockFace::West, [2, 66, 0], before,
).await?;
// PendingやRequiresInspectionを再送しない。隣接階段の更新も独立に待つ。
let progress = client.survival().stair_placement_record().await?;
# let _ = (attempt, progress);
# Ok(())
# }
```

## 対象と原版規則

[共通設置](common-survival-placement.md)と同じ健康なsurvival・通常立位・接地・通常属性・
乾いた既知geometry、実受信のown pose・空cursor・player screen基準を要求する。
静止位置は正確な受信起点に限定する。支持は既存のpassive cubeとfirst outlineの面、
距離は4.5block以下、設置先は面の隣接cellにある完全なair predecessorに限定する。
既存の隣接air設置と同じく、対象cell全体がown bodyの外であることも要求する。
利用側が期待facing・halfやhitを指定する引数はなく、実際のfirst hitからcursorを計算する。
slabの同一cell double化とは別の操作である。

| 入力 | 原版と同じ選択 |
| --- | --- |
| yaw | `Direction.fromYRot`の丸めでsouth/west/north/east |
| 下面クリック | top |
| 側面でhit高さが0.5より上 | top |
| 上面、または側面でhit高さが0.5以下 | bottom |
| 前方に同じhalfで直交する階段があり、反対側の同facing/half階段が妨げない | outer_left/right |
| 上記が成立せず、後方に同じhalfで直交する階段があり、その方向の同facing/half階段が妨げない | inner_left/right |
| その他の既知の配置 | straight |

shape判定はouterを先に検査する。左右は対象のfacingから決める。
材料と隣接階段は乾いたoak familyだけ。水・waterlogged・異種階段・任意datapackの
規則へ広げない。shape参照先は完全なair、propertyなしの既存passive cube、完全なdry oak stairsに
限定し、未対応stateや欠測cellをairまたはstraightに置き換えない。

対象のshapeに加え、水平4近隣の階段を設置後の対象stateで再計算する。
対象・4近隣・その計算に必要なcellをplayer/materialと同じadapter lock境界で読み、
`shape_inputs`に位置と完全なbeforeを保持する。近隣の計算には対象から斜めや2cell先の参照も
必要になり得る。chunk境界でも欠測を補わず、読み取れなければ送信しない。
既存階段のfacing/halfは変更せず、計算中にshapeの固定点を推測する必要はない。

新しい対象の実際の階段collision unionとown body・受信building entityの交差を検査する。
entity資格は[slab](common-slab-placement.md)と同じpose/baby/scaleと有限な固定形状の条件を使う。
これは受信境界での事前確認で、未受信entityやserverの権限・後続変化は保証しない。
支持・姿勢・reach全体の対応範囲をこの追加操作から広げない。

## 独立した受信と保持

`expected`はfacing・half・shape・waterloggedの全propertyを含む。
専用recordの`send`はI/O前intentと完全送信を分ける。
`target_receipt`、ちょうど1個の消費を示す`material_receipt`、実processing ACKを別々に保持する。
最後の1個にはfreshなEmptyが必要で、cache一致や単なるACKは成功にならない。

さらに`neighbor_updates`にはstate変更が必要な各隣接階段のbefore・expectedと
独立したfreshな`receipt`を保持する。変更が不要な近隣には更新packetを要求しない。
必要な隣接receiptが欠けていれば、対象・材料・ACKが揃っていても`Pending`のまま。
完全送信と必要な全受信が揃った場合だけ`ObservedPlaced`になり、既存ledgerの排他が解放される。
受信結果と別actorの変更の因果を識別する保証はない。

未解決中の視点・移動・selection・別操作を拒否する。対象・支持・shape入力・材料の途中の競合、
必要なchunkの置換／unload、world/screen/pose/selectionの変更、取消・部分writeを保持する。
後から元stateやexpectedへ復帰しても競合を消さず、再送やrollbackをしない。
完了はpacket受信時に確定し、後の状態でreceiptや処理ACKを書き換えない。
旧cube・植物置換・slab・stairsのgetterは最後の対応する操作を別々に保持し、接続終了後も読める。

## 根拠と再実行

[原版driver](../scripts/ExportStairPlacement.java)は未改変の`BlockPlaceContext` constructor、
`StairBlock.getStateForPlacement/getStairsShape`、`Direction.fromYRot`、native outline clipを呼ぶ。
cell/fluid参照とcontextの水平方向入力carrierだけを供給し、予期しないworld呼出しは失敗させる。
[source record](../data/client_api/stair_placement_source.json)に公式jar・mappings・classpath・driver・
fixtureのhashを保持する。4近隣のair／8種類のfacing/halfと中心8種類を組み合わせた52,488件、
yawの斜め境界とhit高さ0.5の直前／同値／直後・全6面を含む1,386件、
40種類の完全dry stateに対する軸・対角線・遠方座標のoutline clip 3,200件を照合する。
構成packet試験は受信順、欠測、最後の1個、隣接更新待ち、途中の競合、取消、一度性、履歴を検査する。

```bash
python3 -B scripts/export_stair_placement.py \
  --jar /path/to/1.21.11/server.jar --mappings /path/to/server-mappings.txt \
  --modern-classpath-file /path/to/classpath.txt --work /tmp/stair-oracle --check
python3 -B scripts/run_stair_placement_live.py --accept-eula \
  --server-jar /path/to/1.21.11/server.jar \
  --binary target/debug/examples/climbing_control_probe --output /tmp/stair-live
```

2026-10-23の公式1.21.11実接続は18件を通過した。
4方向それぞれのtop/bottom、上面からのbottomと下面からのtop、inner/outerの左右、
cornerを妨げる階段、既存の隣接階段のstraight→outer/innerと同時に2つの隣接階段が更新される場合を確認した。
各回ちょうど1個のoak stairsを消費し、完全property state、freshなEmpty、processing ACK、
必要な隣接block更新と独立RCONを照合した。誤ったpredecessorは無送信、正しい操作は各1回だけだった。
[source・binary・元packet/RCONのhashと受信結果](evidence/checked-stair-placement-20261023.json)を保持する。
probe・proxy・serverは終了し、元のserver directoryは変更していない。
