# 爆発の受信文脈（Java 1.21.11）

`Client::explosions_after(after_sequence)`は、元の`EXPLOSION` packetを受信順に返す。
`client::explosions`の専用型を使い、既存の`PlayEffect`と`EffectLog`の契約は維持する。
`EventKind::ExplosionReceived`は同じ受信連番の変更通知で、payloadはこのAPIから読む。
Java 1.16.1では`Unsupported`。この観測は送信を行わず、切断後も読める。

```rust,no_run
use voxrig::client::prelude::*;
async fn read(client: &Client) -> Result<()> {
    let log = client.explosions_after(0).await?;
    for record in log.explosions {
        match record.explosion.player_knockback {
            None => println!("player impulseなし"),
            Some(impulse) => println!("元のimpulse: {impulse:?}"),
        }
    }
    let _next = client.explosions_after(log.receive_sequence).await?;
    Ok(())
}
```

`ExplosionObservation`はcenter（3 doubles）、radius（float）、block count（固定長signed INT）、
optional player knockback（bool＋3 doubles）、presentationを分けて保持する。
`None`はpacketでimpulseが省略された事実で、`Some([0.0; 3])`と異なる。
impulseを既存velocityへ代入したり、playerの現在位置から推測したりしない。
block countは元のsigned値であり、破壊位置の一覧やblock変更の確認ではない。

`ExplosionPresentation::Decoded`には主particleのID/options、sound holder、重み付きの
block particle一覧（particle・scaling・speed・weight）がある。weight 0や空一覧も保持する。
soundはregistry IDまたはinlineの名前・optional固定距離で、未受信の名前を補わない。
particle optionsは既存effectsと同じ1.21.11の115要素const tableでdecodeする。
未知のparticle IDやvibration sourceでは、長さのない埋め込みoptionsの境界を特定できない。
この場合は`Undecoded`に主particle IDからpacket末尾までの元bytesを丸ごと残し、
soundなし・空一覧・既知のsimple particleとは扱わない。impulse以前の確定値はそのまま読める。
未decodeのbytes内の切り詰め・末尾余剰・非有限値については検証済みとは主張しない。

`ReceivedExplosion::session`は受信時のconnection・version・world、`receive_sequence`は
接続内の元packet ordinalである。respawn・再設定でも完了済み履歴を消したり、
新worldの所有情報へ書き換えたりしない。通知の`EventLog::cursor`とは別の軸を使う。

- 最新1024件（`MAX_RETAINED_EXPLOSIONS`）、展開後1 packet 16 KiB以下
  （`MAX_EXPLOSION_PACKET_BYTES`）。block countやweightの分だけobjectを増やさない。
  元wire bytesの合計は最大16 MiB相当で、構造体・文字列・item component等の管理領域は別に必要。
- 捨てた標本より古いcursorは`State`、現在受信より先のcursorは`InvalidInput`。
  欠測を空一覧で隠さない。欠測後は履歴が不完全であることを呼び手が扱い、
  現在の受信連番を明示取得して再開する。
- 既知layoutの切り詰め・末尾余剰・不正bool・非有限float/double・負のlist count/weight・
  weight合計のoverflow・過大payloadはprotocol errorで、そのpacketの観測・通知を記録しない。
- 既存の徒歩・飛行・vehicleの移動中断は、impulseなし・ゼロvectorでも維持する。
  このAPIは受信文脈だけを提供し、移動の再開条件を緩和しない。

粒子やsoundの描画・再生、爆発によるblock破壊予測、爆発源がTNT/creeper/風系のどれかを
packetから断定する機能は含まない。packet順はserver tick順・移動適用順の保証ではない。
#44の移動物理残件全体の完了も意味しない。

## Codecと実接続の検証

公式1.21.11 server JAR（SHA-1 `64bb6d763bed0a9f1d632ec347938594144943ed`）の
未変更`ClientboundExplodePacket.STREAM_CODEC`で、impulse省略・明示ゼロ・非ゼロと
registry/inline sound・options付きparticle・重み付き一覧を組み合わせた9構築packetを
同一bytesへround tripした。結果は
[`explosion_packets-1.21.11.json`](../data/client_api/explosion_packets-1.21.11.json)。
これは構築codec fixtureで、実接続のcaptureとは区別する。

`ExportExplosionPackets.java`は公式bundled serverとlibrariesをclasspathへ入れ、
使い捨てdirectoryで再生成できる（class/log出力も同directoryへ置く）。

```bash
javac -proc:none -cp "$ORIGINAL_SERVER_CLASSPATH" -d "$TEMP_OUTPUT" \
  /absolute/path/to/Voxrig/scripts/ExportExplosionPackets.java
java -Xmx512M -XX:ActiveProcessorCount=1 -cp "$TEMP_OUTPUT:$ORIGINAL_SERVER_CLASSPATH" \
  voxrig.explosion.ExportExplosionPackets /tmp/explosion-packets.json
```

実接続consumerは`examples/explosion_observation_probe.rs`。
既存の使い捨てserver driverが`CMD`行をconsoleへ転送し、空きport（25572・25581を除外）を使う。
serverは終了・削除され、元packet traceと観測は出力directoryへ保存される。

```bash
cargo build --locked --example explosion_observation_probe -j2
python3 -B scripts/run_effect_events.py --accept-eula \
  --server-jar /absolute/path/to/server.jar \
  --binary target/debug/examples/explosion_observation_probe --output /tmp/explosion-live
```

2026-10-23に上記consumerでTNT・creeper・wind chargeの近距離／遠距離6ケースを実行した。
各種類で遠距離はimpulse省略、近距離は非ゼロimpulseを受信し、通知とpayloadの受信連番が一致した。
world所有と昇順・切断後の6件保持も確認した。明示ゼロvectorは公式codec構築fixtureで検査し、
liveで受信したとは主張しない。元packetをunit testで再適用してlive観測と照合する。
元bytes・packet順・commands・SDK/binary/JAR hashは
[検証記録](evidence/explosion-observation-20261023.json)へ保存した。
serverはport 35099で起動し、exit 0で終了・削除済み。
