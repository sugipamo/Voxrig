# 共通Clientのブロック狙い判定

`Client::survival().target_block(maximum_distance)`は両版で同じ`BlockTargetObservation`を返す。
`Client::creative().target_block(...)`も同じ型とstatic outlineを使い、受信creative modeを検査する。
共通型は`client::{BlockTargetHit, BlockTargetObservation}`からもimportできる。従来のsurvival importは同じ型。
接続時に版を選び、consumerに版別のraycast型を持たせない。
これは現在の視点から最初に当たる静的block outlineを読む操作で、採掘・設置を送信しない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn target(client: &Client) -> Result<()> {
let observation = client.survival().target_block(4.5).await?;
if let Some(hit) = observation.hit {
    let _ = (hit.position, hit.state, hit.face, hit.point, hit.distance);
}
# Ok(())
# }
```

`maximum_distance`は有限、正、4.5block以下を要求する。既定のsurvival reachに限定し、
reachを変更する属性・effect、fluid、entityの選択は扱わない。
healthyな受信modeとhandleの一致・通常立位・静止・native defaultの移動条件と既知のdry支持を確認する。
creativeでもactive flightからのqueryは現在未対応で、modeを変更したりsurvivalとして扱ったりしない。
実行中/未解決の移動や操作、欠測pose、動くgeometry、未ロード範囲、未対応形状をエラーにする。
`None`は利用可能な全query範囲に静的outlineのhitがなかった場合だけ返す。
未知blockや欠測chunkをairへ変換しない。

返却値には同じadapter lock境界のplayer/inventory、world revision、native standing eyeとquery範囲を保持する。
`hit.state`はその版の完全なnative stateで、block cellとは別にmodel上の交点と面を保持する。
`initial.position`は受信/送信/local cacheの根拠を区別し、最後の実際の位置packetは`received_pose`に残す。
queryはpacket、position代入、在庫変更を行わない。lookによる視点変更は別の明示的操作である。

hitは受信geometryから選んだclient modelの結果であり、serverのtarget receiptや操作許可ではない。
採掘・設置の実行時は、その時点のcontext・first hit・対象・材料を改めて確認する必要がある。
保存済みのquery/JSONを後から実行許可として取り込まない。

## 版別形状と共有処理

視線計算はnativeのfloat角度・float積と、選択版のsine table index丸めを使用する。
DDAによるcell順序、同時crossingの優先順、VoxelShapeの交差、inside判定、auxiliary shapeの面上書きを
共通kernelへ移した。collision boxへのfallbackや近傍blockのprotrusion追加はしない。

1.16.1はdry移動でも検証した12素材とair類に加え、下記7種類のstorage outlineを扱う。
全property variantのnative outline/auxiliary shapeを確認し、grass_blockのsnowy両状態も含む。
1.21.11は既存の静的outlineデータとreconstructionの欠測/移動判定をそのまま接続する。
両版ともchest/trapped_chest/barrel/hopper/dispenser/dropper/ender_chestの全102stateを
未改変公式JARから取得した[storage形状](../data/client_api/storage_outline_source.json)へ接続する。
chestの1/16 inset、double-chestのtype/facing、hopperのoutlineとauxiliaryの違いもnative由来。
block名だけで立方体へ変換せず、完全なpropertiesと版を一致させる。
animated shulkerやworld/block-entity依存形状はこのstate-only表に含めない。
通常立位の支持周辺は両版ともdry cubeに限定するため、任意形状上での立位対応とは別である。
`Feature::BlockTargeting`と従来の`Feature::SurvivalTargeting`はこの制約を含むRestrictedを返す。
storageを狙えることは、その画面が開く、接続されたdouble chestがある、lockや上の障害物がない等の証明ではない。
共通container openの送信・received outcome照合、広いlegacy outline対応、広い採掘・設置条件は残作業である。

## 独立した検証

1.16.1の[oracle fixture](../data/client_api/legacy_targeting_oracle.json)は、SHA-1照合した公式server JARから
[Java verifier](../scripts/VerifyLegacyTargeting.java)で生成する。
Rust実装の出力を期待値として保存する方法は用いない。

- native Entity.calculateViewVector 72ケース。
- 対象12素材の全13stateのoutlineとauxiliary shape。
- native BlockGetter.clip 585ケース。複数cell、軸/対角線、同時crossing、inside、edge付近、短い/零rayを含む。

公式JAR・mappingsとverifier/fixtureのhash、生成条件は[source record](../data/client_api/legacy_targeting_source.json)に残す。
1.16.1のClipContextにはempty-entity constructorがないため、verifierはnativeの5つのdata fieldへ
native vector/OUTLINE/NONE/empty CollisionContextをreflectionで設定する。
視線のpure methodはfieldを読まないため、worldを作らず具体ArmorStandの未初期化instanceへ呼び出す。
交差・traversal・shapeのnative methodは変更しない。このfixtureはentity依存形状や一般的なworld状態の検証ではない。

```bash
java -XX:ActiveProcessorCount=1 -Xmx512M -cp /path/to/1.16.1-server.jar \
  /path/to/Voxrig/scripts/VerifyLegacyTargeting.java /path/to/output.json
```

Java 21のcompiler moduleを含む環境で、作業用ディレクトリから実行する。
server/worldは起動しない。生成JSONをreviewし、source recordのhashも更新する。

1.21.11は共通kernelへ移した後も既存のnative rotation 2,304ケース、outline 25,394ケースと
全state/propertyのshape registry照合を実行する。
同じcommon consumerを両adapter fixtureへ通し、first floor hit/Up面、短いreachでのNone、不正reach拒否、
position/receipt維持を検査する。query自体がpacketを送らないこともnative wireで確認する。
legacyでは未対応block/未ロードchunk/非有限rotation/mode違反の拒否を確認する。

[共通native runner](common-client-native-validation.md)でも同じqueryを公式vanilla両版へ実行する。
RCONは選ばれた対象の実際のstoneと、query前後の実際のPosが変わらないことを確認する。
面/交点計算のoracleは上記native method fixtureであり、RCONのblock確認をserverのhit判定と扱わない。

## Storageの独立した検証

[ExportStorageOutlines.java](../scripts/ExportStorageOutlines.java)は両版の未改変公式JARを使用し、
全102stateのoutline/auxiliaryと各8,976件のnative `BlockGetter.clip OUTLINE/NONE`を取得する。
軸の往復、inset/edge、内部始点、対角線、原点周辺とworld境界付近の座標を含む。
bytecode監査ではchest/hopperはstate propertyからshapeを選び、ender chestは定数、
barrel/dispenser/dropperはbaseのfull cube/empty auxiliaryであることを確認した。
animated shulkerはentityの状態が必要なため今回の静的表から除外する。
JAR/version照合、generator/output hashと監査bytecodeのhashはsource recordに保存する。

```bash
java -XX:ActiveProcessorCount=1 -Xmx1024M -cp /path/to/1.16.1-server.jar \
  /path/to/Voxrig/scripts/ExportStorageOutlines.java 1.16.1 /path/to/legacy-raw.json
# modernのclasspathには展開済みnative server JARとlibrariesを含める。
java -XX:ActiveProcessorCount=1 -Xmx1024M -cp "$VOXRIG_STORAGE_CLASSPATH" \
  /path/to/Voxrig/scripts/ExportStorageOutlines.java 1.21.11 /path/to/modern-raw.json
```

生成JSONの`states`を`storage_outlines-{version}.json`へ、`rays`をdeterministic gzip
（mtime 0）へ保存する。再生成は同じ版・SHA-1照合した入力を使用し、hashも更新する。
world/serverは起動せず、native data carrierと単一cellのBlockGetterから元のclip methodを呼ぶ。
Rustの期待値を生成する方法は用いない。

同じcommon consumerを両adapter・両modeへ通し、全102stateのfirst hit/面/完全properties、
mode違反とactive flight拒否、queryでpacketやpose変更がないことを検証する。
native runnerはcreativeとsurvivalで同じsingle chestを読み、North面のinset交点と
RCONのPos/Rotation不変を照合する。これは共通openの結果確認とは別のread-only検証である。

## 姿勢・scale・reach付きの読取（1.21.11）

`Survival::target_block_with_context()`と`Creative::target_block_with_context()`は、
`ContextualBlockTargetObservation`を返す追加のread-only query。
既存の`target_block(maximum_distance)`と固定寸法の`placement_check`の契約は維持する。

```rust,no_run
use voxrig::client::prelude::*;
# async fn inspect(client: &Client) -> Result<()> {
let observation = client.survival().target_block_with_context().await?;
if let Some(hit) = observation.visibility.value {
    println!("{:?} {:?} distance={}", hit.position, hit.face, hit.distance);
}
# Ok(())
# }
```

同じadapter lockでown pose・scale・block interaction range・位置・回転・支持と
first outlineを捕捉する。Standing/Crouching、scale 0.125〜4の静止・地上に限定する。
位置はこのworldの実受信位置と実受信zero velocityが一致する場合だけ使う。
実行中のcontrolや未解決dispatch、予測pathの位置、乗車、飛行、死亡、motion interruption、
未ロード・移動geometry、再構築不足を拒否する。支持とbody周辺には既存の資格化済みdry形状を使い、
bodyの固体への侵入や支持欠測を拒否する。`support`はcell単位で一意とし、
階段の複数のcollision boxが同じcellを支持しても重複させない。一般的な液体・entity選択やworld依存shapeには拡張しない。
1.16.1では専用のUnsupportedを返す。

| field | 根拠・意味 |
| --- | --- |
| `initial`, `world_revision` | 同一境界のplayer/inventory、位置・回転のsource、受信属性全体とworld cache revision |
| `pose`, `pose_source` | own pose。実際のnative world-entry初期化とmetadata受信ordinalを区別する。欠測pose/sourceは拒否 |
| `scale`, `model_scale` | 実受信base/modifierの計算値へ原版の属性clampを適用したscaleと、寸法に使うfloat変換値 |
| `eye`, `bounds`, `support` | native pose寸法にfloat scaleを掛けて求めるmodel eye・boxと、実際に支持するdry cell |
| `reach` | 実受信block_interaction_rangeの計算値へ原版の0〜64 clampを適用した選択距離。引数による4.5制限は使わない |
| `visibility` | OUTLINE/NONEの実際のfirst hit、または利用可能範囲の完全なmiss。交点・面・完全state・eyeからの距離を保持 |
| `replaceable` | 未評価の`None`。可視性だけでは、材料や設置文脈に依存する置換可否を判断しない |
| `operation_permitted` | 未評価の`None`。read-only queryは採掘・設置・画面openの操作許可を与えない |

scaleとreachには元属性packetの`ValueSource::Received`を残し、未受信のunit scaleや4.5へ
補完しない。元base/modifierとclamp前の値は`initial.attributes`に残る。
eyeとvisibilityは`ValueSource::Predicted`で、受信交点やserverの判定とは区別する。
reach=0も有効で、visibilityはmissになる。既知の非衝突outlineを含むnative traversalを使い、
unknown shapeは手前にある場合に拒否する。手前のhitより後の未知領域を先読みしない。
queryはpacket送信、位置代入、在庫変更を行わず、保存結果を実行許可として再利用しない。

[追加fixture](../data/client_api/contextual_targeting_source.json)は、SHA-1照合した公式1.21.11 JARの
`Avatar.POSES`、`EntityDimensions.scale/makeBoundingBox`、`Entity.calculateViewVector`と
`BlockGetter.clip OUTLINE/NONE`を未改変で呼ぶ。2姿勢・7 scale・4 reach・6回転・4 sceneの
1,344ケースで、寸法とeyeはnative bits、first hitは交点・面で照合する。
角・inside・chest inset・遮蔽・zero reachを含む。inside sceneはclipそのもののfixtureで、
bodyが固体へ侵入した状態をqueryへ許可する証拠ではない。元の25,394 clip fixtureと
両版storage/legacy fixtureも回帰する。

再生成には`scripts/export_contextual_targeting.py`へ`--jar`、`--mappings`、
`--modern-classpath-file`、`--work`を指定する。classpathは公式bundleの元JAR/librariesと照合する。
`--check`は再生成結果とsource hashを比較する。serverは起動しない。
実接続は`scripts/run_contextual_targeting_live.py --binary <probe> --jar <official-jar>
--work <temporary-output> --accept-eula`で再現する。元packet・RCON・native clipの根拠は別々に保持する。

2026-10-23の公式1.21.11実接続は27項目を通過した。
6 scale×4 reachの立位24件、欠測属性拒否、scale=2で低い天井により維持される静止Crouching、
Creative地上の実受信reachを含む。queryの操作packetは0、元属性/pose packetのordinalと
RCONのPos不変、native clip fixtureの面・交点・距離を照合した。
[実行source・binary・元packet/RCONのhashと結果](evidence/contextual-block-targeting-20261023.json)を保持する。

受信packet targetのboxに対するentity rayとblock遮蔽は、
[受信targetのentity query](common-entity-targeting.md)を参照する。
