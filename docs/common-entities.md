# 共通のentity現在状態

`Client::entities()`は、受信した全entity（自分以外）の現在の状態を1回の観測で返す。
1.16.1・1.21.11の両方で使える。値はすべてclientが受信したもので、予測や補間はしない。

| フィールド | 内容 |
| --- | --- |
| `motion` | 種類・出現情報と、最後に受信した位置・向き・速度・接地（既存の`entity_motion`と同じ） |
| `bounding_box` | 種類の既定の大きさを、最後に受信した位置（なければ出現位置）に置いた箱 |
| `health` | 生き物の体力。metadataで受信するまで`None` |
| `equipment` | 装備欄ごとに最後に受信したitem（`MainHand`〜`Head`、1.21.11は`Body`・`Saddle`も） |
| `living` | 生き物（playerを含む）かどうか。1.16.1は出現パケット（生き物用・player用）、1.21.11は種類（`minecraft:player`か既定属性を持つ種類）で決める。種類が分からなければ`None` |
| `metadata` | 最後に受信したentity data（index → 値）。byte・int・long・float・boolean・stringは値を、それ以外の型はnativeの型番号（`Other(n)`）を持つ |
| `metadata_complete` | このentityのentity dataパケットをすべて最後まで読めたか。`false`なら、受信していないフィールドは不明として扱う |

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
