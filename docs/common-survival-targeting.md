# 共通Survivalのブロック狙い判定

`Client::survival().target_block(maximum_distance)`は両版で同じ`BlockTargetObservation`を返す。
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
healthy survival・通常立位・静止・native defaultの移動条件と既知のdry支持を確認する。
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

1.16.1は現段階でdry移動でも検証した12素材とair類のoutlineに限定する。
全property variantのnative outline/auxiliary shapeを確認し、grass_blockのsnowy両状態も含む。
1.21.11は既存の静的outlineデータとreconstructionの欠測/移動判定をそのまま接続する。
通常立位の支持周辺は両版ともdry cubeに限定するため、任意形状上での立位対応とは別である。
`Feature::SurvivalTargeting`はこの制約を含むRestrictedとして返す。
広いlegacy outline対応と採掘・設置の実行・観測の共通化は引き続き残作業である。

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
