# 共通のentity現在状態

過去の受信値が必要な場合は、独立した[entity受信履歴](common-entity-history.md)を使う。

`Client::entities()`は、受信した全entity（自分以外）の現在の状態を1回の観測で返す。
1.16.1・1.21.11の両方で使える。値はすべてclientが受信したもので、予測や補間はしない。

| フィールド | 内容 |
| --- | --- |
| `motion` | 種類・出現情報と、最後に受信した位置・向き・速度・接地（既存の`entity_motion`と同じ） |
| `bounding_box` | 種類の既定の大きさを、最後に受信した位置（なければ出現位置）に置いた箱 |
| `health` | 生き物の体力。metadataで受信するまで`None` |
| `equipment` | 装備欄ごとに最後に受信したitem（`MainHand`〜`Head`、1.21.11は`Body`・`Saddle`も） |
| `living` | 生き物（playerを含む）かどうか。1.16.1は出現パケット（生き物用・player用）、1.21.11は種類（`minecraft:player`か既定属性を持つ種類）で決める。種類が分からなければ`None` |
| `metadata` | 最後に受信したentity data（index → 値）。byte・int・long・float・boolean・string・poseは値を、それ以外の型はnativeの型番号（`Other(n)`）を持つ |
| `scale_attribute` | modern版のscale属性を最後に完全に受信した値。base・modifier・受信順序を保持。未受信・保存上限超過なら`None` |
| `scale_attribute_unavailable_sequence` | 4096byteの個体別保存上限を超えたscale更新の受信順序。古いscaleを無効化し、次の保存可能な更新か新しいspawnで解除 |
| `metadata_complete` | このentityのentity dataパケットをすべて最後まで読めたか。`false`なら、受信していないフィールドは不明として扱う |

## 姿勢の受信

`EntityObservation::pose()`は、原版のpose serializerで受信した最新の値を
`EntityPoseReading`として返す。`pose`は共通名、`native_id`は版固有の番号、
`source`は実際の受信順序を保持する。1.16.1の番号6は`Dying`、1.21.11では
`LongJumping`なので、番号だけを版をまたいで比較しない。

未受信・別のserializer・未知のentity種類では`None`。未知の番号はreadingの
`native_id`を保持し、共通名だけ`None`になる。部分的なmetadataを受信した場合、
既に受信したposeは残るが`metadata_complete`は`false`になる。spawn IDの再利用で
前のentityの姿勢を引き継がない。自分のplayerはこのAPIの対象外。

姿勢の受信は現在のサーバー状態・描画補間・動的な寸法の確認ではない。
`bounding_box`は引き続き種類の既定寸法を使う。

原版serializer全25姿勢の根拠は`data/client_api/entity_pose_oracle.json`。
`scripts/run_entity_pose_oracle.py`で再生成し、`scripts/run_entity_pose_live.py`で
両版の原版サーバーに2クライアントを接続してしゃがみ・解除・削除を検証できる。

## 姿勢・子供・scaleから計算する寸法

`EntityObservation::derived_geometry()`は、player・cow・pig・zombieについて、原版の
姿勢別寸法・子供の寸法・modern版scaleを計算する。既存の`bounding_box`に加えて使える
独立したモデルで、原版931ケースのwidth・height・箱の座標をビット単位で照合している。

```rust,ignore
for entity in client.entities().await?.entities {
    match entity.derived_geometry() {
        Ok(model) => println!("{:?} {:?} {:?}", model.model, model.bounds, model.evidence),
        Err(reason) => println!("dimension model unavailable: {reason:?}"),
    }
}
```

- playerは原版pose enumの寸法を使う。しゃがみは高さ1.5、泳ぎ・滑空・スピンは高さ0.6。
  睡眠とplayerの死亡姿勢は固定の0.2×0.2で、scaleを掛けない。
- cow・pig・zombieはBabyフィールドを使い、子供の寸法を計算する。睡眠は固定寸法。
  年齢の倍率とscaleは原版どおり別々にfloatで掛ける。
- modern版は実際に受信したscaleのbase・modifierを原版の順序で計算し、0.0625〜16に
  clampしてfloatへ変換する。`scale()`で値と入力を読める。未受信を1と推定しない。
  16個を超えるmodifierは4096byteの保存上限内なら`scale_attribute`には残るが、このモデルでは計算しない。
  保存上限を超えた更新では古い値を無効化し、`scale_attribute_unavailable_sequence`に受信順序を残す。
  legacy版にはscale属性がなく、固定の1を使い、attributeは`None`になる。
- `evidence`は位置・pose・Baby・scaleそれぞれの出典とcapture境界を持つ。poseが未受信で
  metadataを完全に読めた場合、原版のStanding既定値を`TypeDefault`として明記する。
  これはpose受信APIの値を補わず、`.pose()`は引き続き未受信なら`None`。
- 未対応の種類、未知pose、別serializer、不明なBaby、未受信scale、未解決の位置は
  `EntityGeometryUnavailable`を返す。部分metadataでは未受信の既定値を使わない。
  明示的に受信済みの必要フィールドだけで計算できる場合は、`metadata_complete:false`を残す。
- 箱は最後に受信した位置targetに置く。量子化誤差は`position_error`に別途保持し、補間・
  現在のserver位置・押し合い・接触・射程の確認として扱わない。移動物理へ自動適用もしない。
  保存したモデルは元のconnection/world/spawn寿命を保持し、削除後の観測には含まれない。

比較fixtureは`data/client_api/entity_dimensions_oracle.json.gz`。
`scripts/run_entity_dimensions_oracle.py`は原版サーバーの登録済みワールド内で原版メソッドを
実行する。`scripts/run_entity_geometry_live.py`は公式両版へ実接続し、受信値と元パケットを照合する。
元JAR・class・パケット・NBTは公開せず、解釈済みの比較結果とhashだけを保存する。
公式両版への57項目の実接続検証と原版931ケースの結果は
[検証記録](evidence/common-entity-geometry-20261011.json)を参照する。

## 制約

- `bounding_box`は既定の大きさだけを使う。姿勢（しゃがみ・寝る等）、`scale`属性、子供の大きさは反映しない。
  種類が分からない場合は`None`。
- `health`は生き物の種類だけ記録する（1.16.1は生き物用の出現パケットで、1.21.11は既定属性の有無で判定）。
  サーバー上の現在の体力ではない。
- 1.21.11のmetadataは、体力（index 9）より前に読めない型があるとそこで止め、体力は更新しない。
  装備も、itemを解読できない欄の手前までを記録する。どちらの場合も接続は切らない。
- 操作に使う`EntityId`は`motion.entity.id`にある。
- `metadata`のindexと型番号は**版ごとに違う**（例: 体力は1.16.1でindex 8、1.21.11でindex 9。posは1.16.1の型18、1.21.11の型20）。
  値ごとに受信連番を持ち、受信したindexだけを更新する。1.21.11は最初に既定値と違うものだけを送る。
- 1.21.11は、読めない型のentryがあるとそこで読むのをやめ、その手前までを記録する（接続は切らない）。
- 2026-10-08に両版の公式serverで、AIなしの豚を出して確認した: `living`は`Some(true)`、体力は`Float(10.0)`
  （1.16.1はindex 8、1.21.11はindex 9）を受信した（`examples/entity_events_probe.rs`）。

## 名前で読むentity data

indexは版ごとに違うので、`EntityObservation::data(EntityDataField)`で名前から読む。
`EntityDataField`は公式サーバーのフィールド名に対応する（例: `CreeperIgnited` は `Creeper.DATA_IS_IGNITED`）。

```rust,ignore
use voxrig::client::{EntityDataField, EntityDataValue};
let entities = client.entities().await?;
let time = client.player_state().await?.world_time.map(|t| t.value.game_time);
for e in &entities.entities {
    if let Some(r) = e.data(EntityDataField::CreeperIgnited) {
        println!("ignited={:?} index={} source={:?}", r.value, r.index, r.source);
    }
    let angry = e.angry(time); // オオカミ・ハチ
}
```

- 返る値は、受信した最新値か、受信していなければその種類の既定値（`EntityDataSource::TypeDefault`）。
  1.21.11のサーバーは既定値と違う値しか送らないため、既定値の補完がないと「falseのまま」と「不明」を区別できない。
  1.16.1は追跡開始時に全値を送るので、通常は受信値になる。
- `None`になるのは: 種類が不明、その版のその種類にフィールドがない（例: 1.16.1の`TicksFrozen`）、
  既定値がworldなしでは決まらない（1.21.11の登録済みvariantなど）、共通層が解読しない型、
  `metadata_complete`が`false`で受信値がない場合。
- `EntityDataField`にないフィールドは`data_by_name("Creeper", "DATA_IS_POWERED")`のように公式の
  （宣言クラス、フィールド名）で読める。宣言クラス名は版で変わることがある（`AgableMob` → `AgeableMob`）。
- `angry(game_time)`は公式の`NeutralMob.isAngry`と同じ判定をする。1.16.1は残り時間 > 0、
  1.21.11は終了時刻 > 0 かつ 終了時刻 − `game_time` > 0。1.21.11では`game_time`が必要で、
  `PlayerObservation::world_time`（約20tickごとに受信）を渡す。
- 1.21.11のentity dataは、生成時に通常spawnと同じbundleで届く。spawnだけ処理された瞬間の観測では既定値が返りうる。

### 確認（2026-10-08、`examples/entity_data_probe.rs`）

両版の公式serverで、AIなしの帯電クリーパー・`Size:2`のスライム・子供のゾンビ・`AngerTime:600`のオオカミ・
普通のオオカミを出して読んだ。

| 版 | クリーパー `MobFlags` / `Powered` / `Ignited` / `SwellDir` | スライム大きさ | ゾンビ`Baby` | オオカミ`angry` |
| --- | --- | --- | --- | --- |
| 1.16.1 | 1 / true / false / -1（すべて受信値、index 14〜17） | 3 | true | true（残り590）/ false |
| 1.21.11 | 1 / true（受信）/ false / -1（この2つは既定値、index 15〜18） | 3 | true | true（終了時刻50833）/ false（既定値 -1） |

1.21.11で受信したindexは、クリーパーが`[9, 15, 17]`（体力・mob flags・帯電）だけだった。

## 派生した multipart 部位（Java 1.16.1）

`EntityObservation::derived_parts()` は、Dragon の親の受信状態から頭・翼のモデルを返す。
`Client::entities()` の受信 entity に部位を追加したり、親の metadata・位置を書き換えたりはしない。
`EntityPartState::Derived` は `target`・`bounds`・`model`・`evidence` を持つ。
`evidence` は親の受信連番と各フィールドの出典であり、部位の spawn 受信や現在の接触を意味しない。

| 部位 | native ID | `model` と条件 |
| --- | --- | --- |
| 頭 | 親 + 1 | phase 5〜7 の `DragonSittingRestPose`。平坦な履歴と旋回補正 0 を仮定する近似 |
| 翼 1 | 親 + 7 | phase 0〜8 の `DragonWingPose`。受信 body yaw で配置 |
| 翼 2 | 親 + 8 | 同上。native の並びで、左右の戦術上の呼称は推定しない |

頭の近似は、着地 phase を受信しただけで実際の native hitbox と一致するとは限らない。
公式 `EnderDragon.aiStep` は頭の配置に位置履歴と `yRotA` も使う。翼も native float の sin/cos 表の丸めや
描画補間は再現しない。`bounds` は命中・射程・視線・安全の証明には使えない。
採用するモデル・接近距離・余裕幅・武器・攻撃時機・反撃条件は利用側が決める。

phase が未受信・不正、死亡 phase 9、受信体力が 0 以下、位置や body rotation が未受信、値が非有限なら
全体が `Err(EntityPartUnavailable)` になる。体力が未受信の場合はモデルを返し、`health_source` は `None` のまま。
既知の飛行 phase は頭だけ `Unavailable(FlyingHead)`、ID の加算があふれる部位は `Unavailable(IdOverflow)`。
phase 10 と Java 1.21.11 は今回のモデルの対象外で、明示的に拒否する。

```rust,ignore
use voxrig::client::EntityPartState;
for parent in &client.entities().await?.entities {
    let Ok(parts) = parent.derived_parts() else { continue; };
    for part in parts {
        if let EntityPartState::Derived { target, bounds, model, evidence } = part.state {
            // model の前提を考慮して利用側が判断する。ここでは観測を表示するだけ。
            println!("{model:?} {bounds:?} {evidence:?}");
            // 攻撃する判断を別途行った場合の一度だけの送信:
            // client.survival().attack_entity_part(target, false).await?;
        }
    }
}
```

`Survival::attack_entity_part` / `Creative::attack_entity_part` は親の connection・world・spawn 寿命と
その時点の受信 phase・体力・モデル可用性を送信前に再確認する。削除後・ID 再利用後の古い対象や、
離陸後の古い頭は拒否する。保存 JSON や整数から live target は構築できない。
送信 receipt は攻撃パケットの送信記録であり、命中や damage の確認ではない。
自動照準・cooldown 待ち・再送・補給は行わない。既存の `attack_entity(EntityId, ...)` はそのまま使える。

### モデルの照合と検証範囲

公式 1.16.1 server.jar（SHA-1 `a412fd69db1f81db3f511c1463fd304675244077`）の
Mojang server mappings で `EnderDragon` → `bac` を確認し、次で bytecode を照合した。

```sh
java -m jdk.jdeps/com.sun.tools.javap.Main -classpath server.jar -c -p bac
```

constructor の部位順序・頭 1×1・翼 4×2 と、`aiStep` の翼 ±4.5 / y+2、頭の長さ 6.5、
sitting 時の `getHeadYOffset` -1 を確認した。頭の履歴・旋回補正は受信だけでは確定できないため、
静止姿勢の近似として明示している。これは End 戦闘成功や server の現在の形状の実測ではない。

`entity::parts` のテストは Golemkit #103 の形状 fixture（`e57f821f`）との一致、受信情報の不変性、
欠損・phase・非有限値・ID overflow・親の寿命を確認する。
native adapter の TCP テストは、実際の spawn/metadata/remove decoder と common API を組み合わせ、
Survival/Creative の送信 ID と action、拒否した対象が送信されないことを確認する。
読み取り専用の `examples/entity_parts_probe.rs` は公式サーバーで親の receipt と部位モデルを観察できる。
今回の同じ SDK ソースと公開 probe を公式 1.16.1 サーバーに接続し、NoAI の Dragon の phase を
6 → 3 → 9 と変更した。頭が近似モデル → `FlyingHead`、全体が `DeadParent` に変わることを確認した。
親の受信出典・モデル・対象 ID を [dragon-parts-validation.json](dragon-parts-validation.json) に保存した。
この確認では攻撃パケットを送っておらず、native 形状の実測や End 戦闘成功を示さない。

最新 main と統合した SDK の追加検証では、`examples/entity_parts_dispatch_probe.rs` と
`scripts/run_entity_parts_dispatch.py` を使い、公式両版へ実接続した。
1.16.1 では Survival / Creative の頭・翼への一度だけの攻撃送信を元パケットと照合し、
誤った mode handle、離陸後の頭、死亡・削除後の部位、別個体の生成後も古い対象、
world 変更後の対象を拒否し、攻撃パケットを書かないことを確認した。
1.21.11 では `UnsupportedVersion` を確認した。
[追加の送信検証記録](evidence/dragon-part-dispatch-20261008.json) は元パケットと対象寿命の検証であり、
NoAI の native 形状との一致、damage、End 戦闘、Golemkit 移行を確認したものではない。

```sh
cargo build --locked --example entity_parts_dispatch_probe
python3 scripts/run_entity_parts_dispatch.py --accept-eula
```

この runner は隔離した localhost の公式サーバーを起動し、終了時に停止する。
検証用の召喚・mode・phase・world 変更は、その一時サーバーだけに対して行う。

Golemkit が固定する SDK `edddce3c` はこの common API より前の版である。
Golemkit の adapter 移行と SDK pin 更新、移行後の実際の組み合わせによる End 検証は別途必要
（[Golemkit #119](https://github.com/sugipamo/Golemkit/issues/119)）。

## データ

1.21.11の種類ごとの大きさと「生き物かどうか」は、公式サーバーから`scripts/ExportEntityDimensions.java`で
書き出した`data/java_1_21_11/entity_dimensions.json`による（出典とhashは`entity_dimensions_source.json`）。
1.16.1は既存の`data/entities.json`を使う。

entity dataのindexと既定値は、両版の公式サーバーから`scripts/ExportEntityData.java`で書き出し、
`scripts/generate_entity_data.py`でMojang名に戻した`data/client_api/entity_data-<版>.json`による
（出典・引数・hashは`entity_data-<版>_source.json`）。constructorを通さずに作った各種類のinstanceで
`defineSynchedData`を呼んで既定値を取る。`Entity`のconstructorが定義する基本フィールドの既定値はそのbytecodeから、
空気量は種類ごとの`getMaxAirSupply()`から取る。1.21.11の9種類（猫・鶏・牛・カエル・絵画・豚・オオカミ・
ゾンビナウティルス・村人ゾンビ）はworldが必要なvariantで定義が止まるため、残りの定数はbytecodeから補い、
定数でないもの（variant・首輪の色など）は不明のままにする。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、`examples/entity_block_probe.rs`で確認した。
consoleから`armor_stand`を出し、頭にダイヤのヘルメットを付け、`data merge`で体力を7にした。

| 版 | `health` | `equipment` | `bounding_box`（x・y範囲） |
| --- | --- | --- | --- |
| 1.16.1 | 7.0 | `Head`: `minecraft:diamond_helmet` | 幅0.5、高さ1.975 |
| 1.21.11 | 7.0 | `Head`: `minecraft:diamond_helmet` | 幅0.5、高さ1.975 |
