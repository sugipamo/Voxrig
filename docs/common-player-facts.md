# 共通の自分の状態（属性・効果・空気・entity ID）

`client.player_state()`の`PlayerObservation`は、位置・体力・inventoryに加えて次を返す。どれも**受信した値**で、両版で同じ名前と型。

| フィールド | 内容 |
| --- | --- |
| `entity_id` | このworldでの自分のnative entity ID（loginまたはrespawnで受け取ったもの） |
| `attributes` | 受信した属性。キーは1.21.11の名前（`minecraft:attack_speed`、`minecraft:movement_speed`など）。1.16.1の`generic.attack_speed`・`horse.jump_strength`・`zombie.spawn_reinforcements`も同じ名前に直す |
| `effects` | 受信した状態効果（`minecraft:speed`など）。削除を受信するまで残す。残り時間は受信時の値で、clientで数え下げない |
| `air_supply` | 受信した空気（tick。満タンは300） |
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
- 攻撃の強さの回復はserver側で数えられる。間隔（`20 / attack_speed`tick）は
  `attributes["minecraft:attack_speed"]`（なければ4.0）から利用側で計算する。
