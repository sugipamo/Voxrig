# 共通Survivalの設置

`Client::survival().place_cube(support, face)`と`placement_record()`は、
1.16.1と1.21.11で共通の`PlacementRecord`を返す。通常の利用側に版分岐は不要。
modern専用の`place_survival_cube`等も維持する。

```rust,no_run
use voxrig::client::prelude::*;
# async fn place(client: &Client) -> Result<()> {
let survival = client.survival();
// 利用側が設置場所・権限・材料を選ぶ。対象がなければ送信しない。
let Some(hit) = survival.target_block(4.5).await?.hit else { return Ok(()); };
let attempt = survival.place_cube(hit.position, hit.face).await?;
// 送信の完了と結果は別。Pendingでも同じ操作を再送しない。
let diagnostics = survival.placement_record().await?;
# let _ = (attempt, diagnostics);
# Ok(())
# }
```

## 前提と送信

健康な通常立位・接地・乾いた既知geometry・通常属性・受信済みsurvival modeとown poseを要求する。
空のcursorと選択中のdefault passive cube stackには実受信の根拠が必要。
player screenへの操作基準は、実受信のwindow 0、または同じ接続・世代・openingに結び付いた
完全な`close_container`送信とする。vanillaがcloseをechoしなくても、収納・通常製作の後に
共通設置へ進める。`initial.inventory.player_screen`にその基準を保持し、受信windowやscreenを
window 0へ書き換えない。再OPEN・ID再利用・不完全なcloseはこの基準を失わせ、
送信前は拒否、設置の実受信待ちでは競合を保持する。modern版固有のchecked設置契約は維持する。
材料は共通dry modelのcubeからgrass blockを除いた範囲。NBT/components付き材料、工具、液体、
未ロード・未知形状はこの入口では扱わない。legacyのgeometryは監査済みのair/12種のcubeに限定する。
effect更新がないことを、serverのeffect一覧が完全に空である証明にはしない。

支持blockと面は4.5block以内の最初のnative outlineと一致し、隣接対象は受信済みairかつ身体の外側であることを検査する。
`cursor`は実際のfirst hitから計算した支持block内の座標で、面の中心を仮定しない。
`initial`はadapterの同じ境界でI/O前に取得したplayer/inventory capture。
`PlacementId`は元sessionとattemptに結び付いたopaqueな識別子で、Deserializeを持たない。
保存したJSONを操作能力として復元しない。

`send.after_sequence`はI/O前の実受信境界、`dispatched`は完全なcommand frameの送信を示す。
legacyは0x2dのmain hand・packed support・face・3つのfloat cursor・inside flagを送る。
modernだけが持つinteraction sequenceをlegacyへ追加しない。
legacyの`interaction_sequence`と`processing`は`None`である。

## 結果と次の操作

| stage | 意味 |
| --- | --- |
| `Pending` | 必要な実受信が揃っていない。再送しない |
| `ObservedPlaced` | 完全送信、新しい対象block、ちょうど1個の材料消費を照合。modernは新しい実processing ACKも要求 |
| `ObservedRejected` | serverが処理した境界が届き、対象・支持・材料がすべて送信前のまま。終端でgateを解放する。理由（保護・entity等）は示さず、再試行の指示でもない |
| `RequiresInspection` | 最初の競合・前提喪失・不確実な送信を保持。後で復元しても消さない |

`PlacementStage`は`#[non_exhaustive]`で、`is_resolved()`は`ObservedPlaced`と`ObservedRejected`でtrueになる。
`rejection`は`ObservedRejected`のときだけ存在し、処理済み境界の受信ordinal（`boundary_receive_sequence`）と、
`before`に等しい新しい対象受信（`target_receipt`）を持つ。

処理済み境界は公式server JARの`ServerGamePacketListenerImpl.handleUseItemOn`に合わせる。
両版とも、早期returnを除くすべての経路の最後で、BlockUpdate(clicked)とBlockUpdate(clicked + face)を
この順に無条件で送る（1.16.1はoffset 358–393、1.21.11はoffset 461–490）。拒否でも送られる。

- 1.16.1にはplacementのACKがない。送信後の単独BlockChange(0x0B)で、支持が`support_state`のまま届き、
  その**直後のpacket**で対象が`before`のまま届いた組を、handlerの終わりとみなす。
  組の両方を`support_receipt`と`target_receipt`に残す。multi-block changeや掘削ACKは組に数えない。
  間に別のpacketが挟まった場合や、材料の受信が変わった場合は`Pending`のまま。
- 1.21.11はACKをhandlerの先頭で登録し、次のconnection tickで送る。
  そのため、handlerが置いた対象は必ずACKより先に届く。
  `before`に等しい新しい対象受信と、intentを覆う新しいACKが両方揃い、
  対象・支持・stairの隣接cell・材料が変わっていなければ拒否とする。`processing`にもそのACKを残す。
  対象のechoがない早期return（client未ロード、無効なitem、距離外、cursor不正）は、
  対象を調べた根拠がないので`Pending`のまま。

拒否は、本当に置かれていないことの保証ではない。終端の後に届いた受信で履歴を書き換えず、
後の変化は`inventory_changes_after`や通常のblock変更として見る。

公式vanilla両版（SHA-1照合済み）で、非OPのsurvival botが、armor standの占める対象cellへ
`place_cube`し、`ObservedRejected`になることを確認した。1.21.11の送信後は
BlockUpdate×2の後にBlockChangedAckが届き、上の順序の前提と一致する。
RCONで対象がairのまま・dirtが5個のままであることを別経路で確認した。
armor standを除いた後、同じ接続で`look`と次の`place_cube`が通り、`ObservedPlaced`
（RCONでdirt設置・4個）になる。
[要約](evidence/placement-rejection-validation-20261027.json)、
[1.16.1報告](evidence/placement-rejection-live-1.16.1-20261027.json.gz)、
[1.21.11報告](evidence/placement-rejection-live-1.21.11-20261027.json.gz)。

```sh
cargo build --locked --features native -j2 --example body_probe
python3 scripts/run_placement_rejection_live.py --accept-eula \
  --binary target/debug/examples/body_probe --jars /path/to/official-jars --output /tmp/placement-rejection
```

`target_receipt`は指定cellの新しい実block update、`material_receipt`は選択slotの新しい実更新である。
一方だけ、無関係なblock更新、cacheの一致、ACKだけでは完了しない。
材料は同じitem ID・名前・dataを持つstackのcountが1減ることを照合し、最後の1個なら明示的なEmptyを要求する。
別actorによる設置・材料の変化と区別できるとは約束しない。
modernの`processing`は実際の処理sequenceとACK packet ordinalで、設置成功そのものではない。

未解決の間は視点・選択・移動・二重設置等を拒否する。
legacyはconnection actorが通常操作を排他し、autonomous physicsを停止する。
実受信が揃った後、または拒否の境界が届いた後、`placement_record()`が同じattemptのgateを解放する。解放packetは送らない。
modernは既存のnative placement guardで同じ前提と結果を検査する。
完了後は新しい場所で次の操作を明示的に開始でき、その場のmode・材料・身体・支持・対象を再検証する。
採掘のair/ABORTによる継続とは別の契約であり、未解決の採掘・移動を設置で解除しない。

完了した対象・材料の受信記録は履歴として保持し、後の現在world/slotと分ける。
閉じた接続でも診断を読み出せる。modernのnative専用操作へ移った場合も直前の共通診断を残す。

## 取消・競合と検証

legacyの保持したcommandはconnection-owned taskが一度だけ実行し、waiter取消で再送しない。
modernのwriter取得前取消では未送信intentを保持し、その後に対象と材料が変化しても送信済みにしない。
partial write・接続終了・一時的な足場喪失・pose/selection/cursor/material変更・chunkの置換等を診断に保持する。
取消・timeoutをrollbackや設置されなかった証明として扱わない。

同じconsumerのpacket試験で、両版の前提capture、freshな対象/材料、競合拒否、取消、一度性、
完了後の次の設置を検査する。modernでは古いACKのordinalを完了根拠にしない。
legacyの特殊Set Slot window -2は、公式1.16.1のInventory送信とInventoryMenu構成へ照合し、
raw inventory番号から共通player screen番号への変換を修正した。通常のwindow番号・cursor・受信ordinalは保持する。

公式vanilla両版では採掘試験後の新しい接続でdirtを設置し、Clientのfresh target/material受信と、
別経路のRCONによる対象dirt・材料3→2・位置不変を照合する。両JVMは順番に実行し正常終了を要求する。
再実行方法・証拠は[共通native検証](common-client-native-validation.md)を参照。
任意形状の設置・container操作・共通fresh recoveryは後続作業に残る。

## 明示した草stateへの追加設置（1.21.11）

`Survival::place_cube_replacing(clicked, face, target, before)`は、監査済みの非液体の草stateを
1個のpassive cubeで置換する。`place_cube`のair限定契約は維持する。
1.16.1では追加操作と専用record読取の両方が明示Unsupportedになる。

```rust,no_run
use voxrig::client::prelude::*;
# async fn replace(client: &Client) -> Result<()> {
let before = voxrig::NativeBlockState {
    name: "minecraft:short_grass".into(),
    properties: Default::default(),
};
// 実際のfirst outlineと、その時点の完全な対象stateを利用側が指定する。
let attempt = client.survival().place_cube_replacing(
    [3, 66, 0], BlockFace::West, [2, 66, 0], before,
).await?;
let progress = client.survival().replaceable_cube_placement_record().await?;
# let _ = (attempt, progress);
# Ok(())
# }
```

対象はpropertiesが空の`short_grass`・`fern`・`dead_bush`だけ。
材料は既存と同じdefault passive cube stackで、草そのもの・components付き材料・液体・雪・蔓・
二段の植物を名前やreplaceable tagだけで一般化しない。通常立位・通常scale・既定reachの既存条件を使う。
新操作の立位周辺走査だけは、この3stateの原版で確認した空のcollisionを許可する。
支持floorを草から作らず、既存の移動や旧設置のdry modelは広げない。

原版`BlockPlaceContext`は、クリックcellの`canBeReplaced(context)`がtrueならそこを選び、
falseなら面の隣を選ぶ。この選択後のcellが明示した`target`・完全な`before`と一致しなければ送信しない。
従って、草をfirst outlineとしてクリックした場合は`target == clicked`であり、
支持cubeの面をクリックして隣の草を置換する場合は`target != clicked`となる。
この限定範囲では、材料の11種の通常BlockItemは対象植物と異なるitemで、
原版baseの置換・設置state処理を継承する。特殊item・hit位置・secondary useに依存する別blockの規則へ
推測で拡張しない。first outline、実cursor、対象predecessor、材料、player captureを同じlock境界で固定する。
位置の不確かさが草のfirst outlineを保証できない場合も拒否する。

`ReplaceableCubePlacementRecord`には`first_outline`と`replaces_clicked`、元の完全な支持・対象state、
材料、送信前intent、独立したblock/material/processing受信を保持する。
`PlacementStage`の意味は既存と同じ。blockだけ、消費だけ、ACKだけ、cache一致では完了しない。
最後の1個はfreshなEmptyを要求する。クリックcellを置換する場合は、同じcellのexpected cubeへの
更新を支持喪失とは扱わないが、別stateへの変更やexpected受信後の復帰は競合を保持する。
取消・部分write・screen/selection/world/pose変更・chunk置換で再送しない。
専用recordは完了後のblock・在庫・ACKで書き換えず、接続終了後も読める。
旧`placement_record()`と新しい専用getterは、それぞれ最後の対応する操作を保持する。

原版fixtureは[生成driver](../scripts/ExportReplaceablePlacement.java)と
[source record](../data/client_api/replaceable_placement_source.json)に基づく6,048ケース。
3植物と拒否範囲のstate、11材料と同じ植物item、全6面、secondary use両値、hit高さ3種、
クリックcell／隣接cellを照合する。原版のcontext・`canBeReplaced`・`getStateForPlacement`を未改変で呼び、
3植物の空collision・非液体・outline・auxiliaryも確認する。
新操作専用のconst shape tableは、原版clipの別384ケースで、境界・内部・対角線・遠方座標の
first outline／面／交点を照合する。既存queryの未対応shapeを一般化しない。cellだけを返すLevelとsecondary useの入力carrierを使い、
collisionの世界全体の判定やserverの設置許可をfixtureから主張しない。

```bash
python3 -B scripts/export_replaceable_placement.py \
  --jar /path/to/1.21.11/server.jar --mappings /path/to/server-mappings.txt \
  --modern-classpath-file /path/to/classpath.txt --work /tmp/replacement-oracle --check
python3 -B scripts/run_replaceable_placement_live.py --accept-eula \
  --server-jar /path/to/1.21.11/server.jar \
  --binary target/debug/examples/climbing_control_probe --output /tmp/replacement-live
```

構成packet試験は独立した受信順、欠測、最後の1個、途中の対象・支持・材料変更、chunk変更、
取消した未送信intent、一度性、完了後の履歴を検査する。

2026-10-23の公式1.21.11実接続は3植物×2経路の6件を通過した。
各回1個だけのdirtを消費し、freshな対象dirt・選択slotのEmpty・processing ACKを独立に照合した。
RCONでも対象dirtとSelectedItem消失を確認し、旧air限定APIと誤ったpredecessorでは設置packetは0、
正しい追加APIでは1回だけだった。vanillaはslot同期よりsection更新が後になる場合があるため、
runnerは両方の実受信predecessorが揃うまで待つ。
[source・binary・元packet/RCONのhashと受信結果](evidence/replaceable-cube-placement-20261023.json)を保持する。
起動したprobe・proxy・serverは終了し、元のserver directoryは変更していない。

乾いたoak slabのtop/bottom/doubleは追加の[`place_slab`](common-slab-placement.md)で扱う。
専用recordに完全property stateと実際の対象cellを保持する。

乾いたoak stairsのfacing/half/inner/outerは追加の[`place_stair`](common-stair-placement.md)で扱う。
対象・材料・ACKに加え、必要な隣接階段のshape更新も専用recordで確認する。

水源・流水・wet oak slabを明示する追加入口は
[`place_slab_with_fluid`](common-fluid-slab-placement.md)で扱う。
原版のfluid型に従いwaterloggedを選び、周辺流れの収束は設置の成功条件に含めない。
