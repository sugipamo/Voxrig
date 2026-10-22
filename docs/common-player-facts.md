# 共通の自分の状態（属性・効果・空気・entity ID）

`client.player_state()`の`PlayerObservation`は、位置・体力・inventoryに加えて次を返す。どれも**受信した値**で、両版で同じ名前と型。

| フィールド | 内容 |
| --- | --- |
| `entity_id` | このworldでの自分のnative entity ID（loginまたはrespawnで受け取ったもの） |
| `attributes` | 受信した属性。キーは1.21.11の名前（`minecraft:attack_speed`、`minecraft:movement_speed`など）。1.16.1の`generic.attack_speed`・`horse.jump_strength`・`zombie.spawn_reinforcements`も同じ名前に直す |
| `effects` | 受信した状態効果（`minecraft:speed`など）。削除を受信するまで残す。残り時間は受信時の値で、clientで数え下げない |
| `air_supply` | 受信した空気（tick。満タンは300） |
| `world_time` | 最後に受信したworldの時刻（`game_time`・`day_time`）。serverは約20tickごとに送るので、現在のgame timeはこれより最大でその分進んでいる。respawnでも消さない |
| `using_item` | 受信したアイテムの使用中（[アイテム使用](common-item-use.md)） |

## 属性

`PlayerAttribute`は受信した`base`と`modifiers`（受信順）と、`value`を持つ。
`value`は公式の`AttributeInstance.calculateValue`と同じ順序（加算→基準値の倍率→全体の倍率、各版のmapの反復順）で
計算したもので、属性ごとの範囲への丸め（clamp）はしていない。移動速度と同じ計算で、[比較基準](movement-oracle.md)で検証済み。

- serverは、値が変わった属性だけを送る。手に持った剣の修飾子などがなければ`attack_speed`は送られず、
  公式clientは自分の既定値（`attack_speed`は4.0）を使う。この表にない属性は「clientの既定値のまま」と読む。
- 2026-10-08に両版の公式serverで確認した: 素手では`attack_speed`は送られず、鉄の剣を選ぶと
  `base 4.0`、修飾子`-2.4000000953674316`（加算）、`value 1.5999999046325684`を受信した
  （1.16.1の修飾子IDは公式の`fa233e1c-4180-4865-b01b-bcce9785aca3`）。
- respawnやworldの変化で、属性・効果・空気はいったん空になり、受信し直すまで出さない。
  公式clientは次元の移動などで属性を引き継ぐことがあるが、Voxrigは受信していない値を推測しない。

## 攻撃の間隔と腕振り

- `attack_entity`は攻撃のpacketを1つ送るだけで、待たない・腕を振らない。
- 公式clientは攻撃の直後に主の手を振る。同じにするには続けて`swing_arm(Hand::Main)`を送る。
- Java 1.21.11の`client.attack_strength().await?`は、ローカルの攻撃強度を`f32`の
  `0.0..=1.0`で返す。1.0ならローカルで全回復。送信や自動待機は行わず、戦闘方針は利用側が持つ。
  nativeの`Operations::attack_strength()`も同じ値を返す。1.16.1の共通accessorは`Unsupported`。
- 接続内の単調時計で50msごとの整数tickを数える。partial tickは0とし、
  `delay = ((1.0 / attack_speed) * 20.0) as f32`として、
  `clamp(ticks_since_reset / delay, 0, 1)`を使う。除算・乗算・f32変換の順序もvanillaに合わせる。
  world login/respawn・再設定で0に戻す。未ready・切断・revoke時はエラー。
- 送信が完了した攻撃と腕振り（両手）、選択slotの変更で0に戻す。拒否した操作や送信前の取消は
  戻さない。同じslotの再選択は戻さない。受信による選択slot変更と、現在の主の手のitem ID変更も
  戻す。countやcomponentだけの変更、別slotの更新、属性packet自体では戻さない。
  送信前に取り消した選択試行は、受信itemを判定するslotにも使わない。
  未受信slotと受信emptyは区別し、新たに主の手の内容が判明した場合も戻す。
  画面の開閉などでinventoryの観測が欠けても、最後に分かっていたitem IDを保持する。
  欠測への変化や同じitemの再受信では戻さず、異なるitem・emptyの受信では戻す。
- 速度は受信した自分の`attack_speed`のbaseと全modifierから、既存の版別属性計算で求め、
  0..1024へclampする。未受信ならvanillaの既定4.0。速度0では回復しない。
  受信modifierに装備の`minecraft:base_attack_speed`も含まれるため、item表の速度を重ねて加算しない。
  手持ち変更後に新しい属性が届くまでは直前の属性を使い、item componentからの先行予測は行わない。
- これはサーバーの攻撃強度・ダメージ・命中のACKではない。server lag、tick rate変更、
  属性packetの遅延でserver側と差が出る。`attack_entity`は全回復を待たずに従来どおり送信する。
  Microsoft認証・署名はこの実装のスコープ外。

```rust,no_run
# async fn attack(client: &voxrig::Client, target: voxrig::client::EntityId) -> voxrig::Result<()> {
while client.attack_strength().await? < 1.0 {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}
client.survival().attack_entity(target, false).await?;
# Ok(())
# }
```

constructed packetの単体試験で属性の加算・倍率、剣の速度、全回復の丸め境界、0/範囲外、他entity、切断入力、
slot/item変更・再設定を確認する。TCP試験で攻撃・腕振り・選択の送信後のreset、
拒否・送信前取消、読み取りの無送信を確認する。今回の実server検証は未実施。

## 目が水中か

`client.eye_in_water()`は、公式の`Entity.isEyeInFluid(WATER)`と同じ判定を返す（溺れ・水中採掘の遅さなどの条件）。

- control sessionが動いている間は、その最新の予測frame（`ControlFrame::eye_in_water`）を返す。予測の姿勢（泳ぎ・しゃがみ）に従う。
- それ以外は、受信した位置と受信したブロックから、立った目の高さ（1.62）で計算する。control sessionの外でのしゃがみ・泳ぎの姿勢は反映しない。
- 位置や目の位置のブロックが分からなければ`None`。
- 2026-10-08に両版の公式serverで、乾いた場所で`Some(false)`、目の高さのブロックを水源にすると`Some(true)`になった
  （`examples/entity_data_probe.rs`）。
