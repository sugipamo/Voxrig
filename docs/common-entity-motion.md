# 共通entityの受信motion観測

`Client::entity_motion(EntityId)`は、`entity_spawns()`で取得した同じspawn寿命の
最新の空間情報を返す。Survival／Creativeの両方で同じ観測APIを使う。
乗車中も`MountId::vehicle()`で得た元のentityを指定できる。

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(client: &Client) -> Result<()> {
    for spawn in client.entity_spawns().await?.entities {
        let motion = client.entity_motion(spawn.id).await?;
        if let Some(position) = motion.position {
            println!("{:?} {:?}", position.value.position, position.source);
        }
    }
    Ok(())
}
```

`entity.spawn_position`は元のspawn packetの履歴であり、後続の移動によって変えない。
`position`は受信packetから復元した位置target、`rotation`はbody yaw/pitch、
`head_yaw`は明示された頭の回転、`velocity`は最後に供給された速度sample。
それぞれのfieldに独立した受信ordinalを付ける。captureの`receive_sequence`で
古いfieldを新しく受信したことにはしない。速度がゼロでも現在の停止や操作完了を証明しない。
`on_ground`も受信flagであり、独立した支持・衝突判定ではない。

同じnumeric IDが再利用されても古い`EntityId`は失効する。別接続・別world・
破棄済みspawnも拒否する。未spawnのentityへの更新は、後のspawnへ繰り越さない。
乗車位置packetにはentity IDがないため、受信時点の実乗車関係と、そのとき既知だった
spawnが一致する場合だけ反映する。乗車後に遅れて来たspawnをretroactively結び付けない。

## バージョン差

1.16.1はfloorによる4096単位の座標変換、1.21.11はネイティブVecDeltaCodecの
Java Math.roundとzero-delta保持に従う。相対座標には各axisで1/4096 blockの
保守的なquantization boundを付ける。絶対座標packetではboundはゼロ。
1.16.1のpainting spawnはblock anchorなので、entityのfeet baselineを作らない。
経験値orb・player spawnに存在しない速度やhead yawをゼロで補わない。

1.21.11のteleportは`correction`へ元のposition／delta／rotation／flagsを保存する。
相対fieldのnative interpolation baselineは履歴から得られないため、関係する
position／rotation／velocityを未解決にする。通常のrelative移動のcodec baselineは
teleportではresetせず、native position syncでresetする。

これは受信targetの観測であり、entity physicsやrender interpolationではない。
metadata・health・equipment・hitbox・新minecartの特殊補間は別の残作業。
下車後の安全な地上操作継続も、このAPIだけでは許可しない。

## 検証資料

[original codec corpus](../data/client_api/entity_motion_packets.json)には両版それぞれ
64例の位置変換と、1.16.1 base MoveEntity packetがentity IDだけであることを保存した。
[source record](../data/client_api/entity_motion_source.json)に公式server JAR・mappings・
元classpath・generator・出力のhashを記録している。World・game handler・codec本体を置換していない。


同じcommon consumerの`vehicle-control`シナリオで、両版・両modeの乗車→有限入力→
fresh受信位置の変化→実下車→切断を検証した。元spawnは不変で、各位置fieldの受信ordinalが
進むことを確認した。独立RCONは元UUIDの乗車、車両移動と下車を確認する。
[固定入力と4ケースの結果](evidence/common-entity-motion-20261006.json)に、sourceとconsumer、
元JAR、受信fieldと元traceの証拠を保存している。異なる観測時点のRCON座標との完全一致は要求しない。
