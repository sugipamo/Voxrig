# 共通のentity現在状態

`Client::entities()`は、受信した全entity（自分以外）の現在の状態を1回の観測で返す。
1.16.1・1.21.11の両方で使える。値はすべてclientが受信したもので、予測や補間はしない。

| フィールド | 内容 |
| --- | --- |
| `motion` | 種類・出現情報と、最後に受信した位置・向き・速度・接地（既存の`entity_motion`と同じ） |
| `bounding_box` | 種類の既定の大きさを、最後に受信した位置（なければ出現位置）に置いた箱 |
| `health` | 生き物の体力。metadataで受信するまで`None` |
| `equipment` | 装備欄ごとに最後に受信したitem（`MainHand`〜`Head`、1.21.11は`Body`・`Saddle`も） |

## 制約

- `bounding_box`は既定の大きさだけを使う。姿勢（しゃがみ・寝る等）、`scale`属性、子供の大きさは反映しない。
  種類が分からない場合は`None`。
- `health`は生き物の種類だけ記録する（1.16.1は生き物用の出現パケットで、1.21.11は既定属性の有無で判定）。
  サーバー上の現在の体力ではない。
- 1.21.11のmetadataは、体力（index 9）より前に読めない型があるとそこで止め、体力は更新しない。
  装備も、itemを解読できない欄の手前までを記録する。どちらの場合も接続は切らない。
- 操作に使う`EntityId`は`motion.entity.id`にある。

## データ

1.21.11の種類ごとの大きさと「生き物かどうか」は、公式サーバーから`scripts/ExportEntityDimensions.java`で
書き出した`data/java_1_21_11/entity_dimensions.json`による（出典とhashは`entity_dimensions_source.json`）。
1.16.1は既存の`data/entities.json`を使う。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、`examples/entity_block_probe.rs`で確認した。
consoleから`armor_stand`を出し、頭にダイヤのヘルメットを付け、`data merge`で体力を7にした。

| 版 | `health` | `equipment` | `bounding_box`（x・y範囲） |
| --- | --- | --- | --- |
| 1.16.1 | 7.0 | `Head`: `minecraft:diamond_helmet` | 幅0.5、高さ1.975 |
| 1.21.11 | 7.0 | `Head`: `minecraft:diamond_helmet` | 幅0.5、高さ1.975 |
