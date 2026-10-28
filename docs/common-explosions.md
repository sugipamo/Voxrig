# 爆発の受信文脈（Java 1.16.1 / 1.21.11）

`Client::explosions_after(after_sequence)`は、元の`EXPLOSION` packetを受信順に返す。
`client::explosions`の専用型を使い、既存の`PlayEffect`と`EffectLog`の契約は維持する。
`EventKind::ExplosionReceived`は同じ受信連番の変更通知で、payloadはこのAPIから読む。
両版で利用できる。この観測は送信を行わず、切断後も読める。

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

Java 1.21.11の`ExplosionObservation`はcenter（3 doubles）、radius（float）、block count（固定長signed INT）、
optional player knockback（bool＋3 doubles）、presentationを分けて保持する。
`None`はpacketでimpulseが省略された事実で、`Some([0.0; 3])`と異なる。
impulseを既存velocityへ代入したり、playerの現在位置から推測したりしない。
block countは元のsigned値であり、破壊位置の一覧やblock変更の確認ではない。

Java 1.21.11の`ExplosionPresentation::Decoded`には主particleのID/options、sound holder、重み付きの
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

- 最新1024件（`MAX_RETAINED_EXPLOSIONS`）。Java 1.21.11は展開後1 packet 16 KiB以下
  （`MAX_EXPLOSION_PACKET_BYTES`）。block countやweightの分だけobjectを増やさない。
  元wire bytesの合計は最大16 MiB相当で、構造体・文字列・item component等の管理領域は別に必要。
- 捨てた標本より古いcursorは`State`、現在受信より先のcursorは`InvalidInput`。
  欠測を空一覧で隠さない。欠測後は履歴が不完全であることを呼び手が扱い、
  現在の受信連番を明示取得して再開する。
- Java 1.21.11の既知layoutの切り詰め・末尾余剰・不正bool・非有限float/double・負のlist count/weight・
  weight合計のoverflow・過大payloadはprotocol errorで、そのpacketの観測・通知を記録しない。
- 1.21.11の現在のGround owner（Survival／Adventure／Creative）は、decode済みpacketの
  optional impulseをreaderの状態ロック内で一度だけ取り込む。省略は加算なし、明示ゼロはゼロの加算。
  Flight／Glide／vehicle、ownerなし、world変更、不確かなwrite、未decodeの表現では
  従来のpacket ID・受信連番付き中断を維持する。

粒子やsoundの描画・再生、爆発によるblock破壊予測、爆発源がTNT/creeper/風系のどれかを
packetから断定する機能は含まない。受信順でのモデル適用はserver tick時計の再現ではない。
#44の移動物理残件全体の完了も意味しない。

## Java 1.16.1 の地形変更通知と履歴

1.16.1のcenterは元の3 floatをf64へ損失なく拡張した値、radiusはfloat、
block countは位置一覧の長さである。`player_knockback`は必須の3 floatを拡張した
`Some`（明示ゼロも含む）。`ExplosionPresentation::Java1_16_1 { affected_blocks }`は、
centerの各軸をfloorした座標へ元のsigned byte offsetを加えた絶対位置を保持する。
重複とpacket内順序を保ち、未送信のparticle/soundを補わない。
このvariantは表現情報に限らず、旧版の実際の地形適用情報を含む。
`ExplosionPresentation`は既に`non_exhaustive`なので、既存利用者の`_`分岐を維持して追加対応できる。

受信処理はworldの該当位置をairへ更新し、同じcoherent-state gate内で履歴を保存して
`BlocksChanged { min, max }`と`ExplosionReceived`を発行する。両通知と履歴の
`receive_sequence`は同じ。boundsは一覧の各位置を含むinclusiveな直方体であり、
範囲内の全セルが破壊されたという意味ではない。未ロード座標を受けても既知地形を創作しない。
空一覧では`ExplosionReceived`だけを発行する。イベントの再読・履歴の再読で
地形適用や運動量加算を繰り返さない。既存の運動中断方針や送信の拒否条件は変更しない。

旧版の位置一覧には件数に加え、接続全体で合計65,536位置の保持上限
（`MAX_RETAINED_EXPLOSION_BLOCKS`）がある。超えると古いpacketから全体をevictする。
単独で上限を超えるpacketも受信・地形適用・共通通知を行い、その履歴を保持できなかった
ことを`State`で知らせる。途中までの一覧を完全な履歴として返さず、新たなprotocol拒否も加えない。
既存decoderの受理範囲は維持する。16 KiBの制限は1.21.11専用である。
共通イベントは独立した4096件の保持窓を使い、こちらの欠落も`events_after`の`State`になる。

情報の所有はサーバーを正とし、Voxrig worldは受信済み地形の唯一の現在値を持つ。
上位が複製した地形・遮蔽・経路の前提は、同じ受信境界の`BlocksChanged`で捨てて
対象領域を再観測する（領域の体積に比例するコスト）。履歴欠落やworld変更でも旧前提を撤回する。
最新worldの再読から過去の爆発位置・運動量は再導出できないため、元文脈は専用の有界履歴を使う。

fixtureは`explosion_notification_tests`。公式1.16.1 JARのSHA-1を検査する既存の
`run_climbing_control` harnessで、TNTの非空一覧、creeperの空一覧、原packetとの一致、
通知の受信連番、全位置のworld上のairおよびRCON上のair、切断後の履歴を確認する。
保持欠落はfixtureでpacket上限・位置上限・共通イベント上限を確認する。

```sh
cargo build --locked --example explosion_changes_probe
python3 -B scripts/run_explosion_changes_live.py --accept-eula \
  --binary target/debug/examples/explosion_changes_probe \
  --jars /path/to/official-jars --output .local/explosion-changes-live
```

実接続証跡: [1.16.1 の検証記録](evidence/explosion-changes-live-1.16.1-20261027.json.gz)。
stable / Rust 1.85.0 × default / native の fmt・Clippy・全テストの
[検証結果](evidence/ns01-validation-20261027.json)も保存している。

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


## Ground ownerへの適用

`ClientPacketListener.handleExplosion`はoptional値を`Entity.addDeltaMovement`へ渡す。
`ENTITY_VELOCITY`の`lerpMotion`は速度を置き換える。この違いをprivateな順序付き入力へ反映し、
連続爆発、速度置換、補正をreaderの受信順で適用する。相対速度補正の基準には、
同じworld・同じGroundモデルの現在速度を使う。受信したvelocity sampleを予測値で上書きしない。
`ControlRecord::velocity_updates`は元の速度受信の計数で、爆発の回数には使わない。
加算した爆発は`ControlRecord::explosion_impulses`に数える（Java 1.16.1の実行中Ground ownerも同じ）。
爆発の元値・受信順・worldは`explosions_after`で読む。getterを繰り返しても排出・再適用しない。

正常な明示停止後に残る同じモデルへ届いた新しいimpulseも一度だけ取り込み、再開はその速度を継ぐ。
停止待ちfutureの取消で接続所有taskを取り消さず、owner交代後に古い入力を再配送しない。
別の有限移動・飛行・vehicle ownerの受付済みintentも検査し、最初のwrite前に残る旧Groundモデルへ渡さない。
未知のモデル、死亡、別の位置送信、部分write、world変更では旧impulseから再開しない。
地形等でtickを予測できなければ既存のPaused診断を残し、加算を繰り返さない。
爆発直後のモデル速度を「受信zero velocityの静止証明」に使わない。

独立値は`explosion_ground_oracle.json.gz`の10場面・240 tick。
公式clientのhandlerと呼出先をfingerprintで結び、未変更の`addDeltaMovement`／`lerpMotion`と
公式Playerのtravelを実行した。地上／空中の連続二爆発、速度通知の前／間／後、省略と明示ゼロを含む。
構築packetの試験はその全位置・速度・接地を照合し、補正、読取、取消、死亡、owner／world／writeの
境界を別に検査する。既存の全1,024ケースの補正oracleと両版の移動fixtureも回帰対象。
入力解除の部分writeとflush失敗は実際のadapter writerで発生させ、後続の爆発が旧モデルへ
加算されず、停止理由とpacket ID・受信連番付き中断、元の観測履歴が残ることを検査する。

```bash
python3 -B scripts/movement_oracle/explosion_scenarios.py /tmp/explosion-scenarios.json
python3 -B scripts/run_explosion_ground_oracle.py \
  --downloads /absolute/path/to/downloads \
  --work /tmp/explosion-ground-oracle --scenarios /tmp/explosion-scenarios.json
cargo test --locked --lib -j2 explosion -- --test-threads=2
cargo build --locked --example climbing_control_probe -j2
python3 -B scripts/run_explosion_ground_live.py --accept-eula \
  --jars /absolute/path/to/downloads --binary target/debug/examples/climbing_control_probe \
  --work /tmp/explosion-ground-live
```

公式1.21.11 serverの使い捨て接続では、歩行中／空中の爆発、停止・再開、実補正とその後の爆発を
検査する。元packet、モデルframe、RCON座標は別の根拠として記録する。
これは原版clientのpacket時計やtick単位のserver速度一致を証明するものではない。


2026-10-23の実接続は13項目PASS、traceエラー0件。
歩行中と明示jumpによる空中の爆発、停止・再開の最初の移動、履歴の非排出を確認した。
実補正では既存の接触origin失効による停止診断を維持し、新ownerで補正後の基準から再開した。
その後の爆発も一度だけ適用した。port 38107のserverは停止済み。
元packetとRCON座標、runtime source／binary／JAR hash、検証fixture修正の試行は
[実接続証跡](evidence/common-explosion-ground-20261023.json)へ保存した。

### Java 1.16.1 の実行中owner（audit P10）

1.16.1のnative受信は従来どおり互換モデルの速度へimpulseを加算する。実行中の共通PlayerControlは
毎tick自分のframeで互換モデルを上書きするため、以前はimpulseが次の移動に届かなかった
（`control_explosion_tests`は修正前のmainで、立ったままのownerがy=65.0から動かないことを再現した）。
現在は受信処理が同じ受信連番で実行中のGroundモデルへ一度だけ加算し、`explosion_impulses`に数える。
Flight／Glideの実行中ownerは、1.21.11と同じ理由で次のtickに何も送らず停止する。
モデルの位置・速度が有限でなくなる加算も同じ理由で停止する。新しい拒否条件や再送はない。

```bash
cargo build --locked --example climbing_control_probe -j2
flock ~/logs/voxrig-live.lock python3 -B scripts/run_control_explosion_live.py --accept-eula \
  --jars /absolute/path/to/downloads --binary target/debug/examples/climbing_control_probe \
  --work /tmp/control-explosion-live
```

2026-10-28の公式1.16.1 serverの使い捨て接続（creeper、`ExplosionRadius:2`、mobGriefing無効）では、
立ったままのGround ownerがknockback `[-0.425, 0.459, 0]`を受信した。その後の7個の移動パケットが
x 0.075→-1.038、y 65.38→66.06（頂点）→65.91と弧を描き、`explosion_impulses=1`、`corrections=0`だった。
着地後のRCON座標とモデルframeが一致し、二度目の爆発で2に増えた。停止記録も2を保つ。
同じbinaryで既存の1.21.11 Ground実接続（13項目）も再実行してPASSし、各段階の
`explosion_impulses`（1→2、再開後0、補正後の新ownerで1）を記録した。元packet・frame・RCON座標と
source／binary／JAR hashは[実接続証跡](evidence/control-explosion-live-20261028.json.gz)へ保存した。
