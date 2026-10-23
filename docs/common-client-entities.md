# 共通Clientの基本装備とentity操作

`Client::entity_spawns()`は、現在の接続・worldで受信したspawnのうち、まだdespawnを受信していない
entityを返す。両版で同じ`EntitySpawns`／`EntitySpawn`／`EntityId`を使う。
`spawn_position`はspawn packetが供給した元の座標と受信ordinalを保ち、後続の移動やmetadataで
更新された現在位置とは扱わない。1.16.1のpaintingはblock anchor、experience orbはUUIDなし。
既知のentity typeは版に結び付いたbuiltin registry IDと名前へ解決する。
未知typeは元のnative type IDを残し、別の既知entityへ読み替えない。

```rust,no_run
use voxrig::client::prelude::*;
use voxrig::client::Hand;
# async fn run(client: &Client) -> Result<()> {
let received = client.entity_spawns().await?;
// 選択、距離、遮蔽、装備、戦術は利用側で判断する。
if let Some(entity) = received.entities.iter().find(|e| e.type_name.as_deref() == Some("minecraft:villager")) {
    let dispatch = client.survival().interact_entity(entity.id, Hand::Main, false).await?;
    // dispatchは一つのnative frameの送信完了。画面が開いたという応答ではない。
    let screen = client.screen_state().await?;
    println!("dispatch={dispatch:?}, received screen={screen:?}");
}
# Ok(())
# }
```

両mode handleに`interact_entity(target, hand, sneaking)`と`attack_entity(target, sneaking)`がある。
受信modeと接続の既存mutation契約を検査し、元の`EntityId`が同じ接続・world・spawn ordinalに
属することを送信前に検査する。削除済みIDのほか、同じ数値IDやUUIDで再spawnした対象も古いIDでは
操作できない。`EntityId`は整数や保存JSONから生成できず、record/replayを実行権限へ変換しない。

操作は一回のnative INTERACT／ATTACK frameを送る。自動選択、aim、reach/visibility検証、cooldown待ち、
arm swing、retry、health/metadataの正規化は供給しない。
返る`DispatchReceipt`は完全送信だけを表し、damageや画面表示、取引の成功を保証しない。
別途実受信またはサーバー側の結果を確認する。画面OPEN自体にもentityへの因果ACKはない。

1.21.11では両mode handleに`interact_entity_at(target, hit, hand, sneaking)`もある。
`target`は実受信した`EntityId`、`hit: [f32; 3]`はentity原点からの相対座標で、world座標ではない。
全成分の有限性と通常interactionと同じmode・mutation admission・接続/world/spawn寿命を検査する。
有限座標をhitbox内へ制限したり、ray queryや現在位置を必要条件にしたりしない。
版別encoderで一回のINTERACT_AT frameを送り、同じ`DispatchReceipt`で完全送信のみを返す。
reach/visibility、装備交換・取引などの結果は保証せず、別途確認する。
共通APIの1.16.1は`Unsupported`。既存nativeの位置指定操作や通常`interact_entity`の契約は変えない。

```rust,no_run
use voxrig::client::{Client, EntityId, Hand};
# async fn run(client: &Client, stand: EntityId) -> voxrig::Result<()> {
let dispatch = client.survival()
    .interact_entity_at(stand, [0.0, 0.1, 0.0], Hand::Main, false).await?;
// armor standの足元への要求。装備を取得できたかは別途実受信等で確認する。
# let _ = dispatch;
# Ok(())
# }
```

キャンセル時のnative sender契約も維持する。1.16.1はactorへ渡す前にpending dispatchを保持し、
不確実なキャンセル後は再送せず再接続を要求する。1.21.11はwriter待ちのキャンセルではframeを
書かず、frame書込途中のキャンセル・失敗では接続を閉じる。通常の完全送信後にも自動再試行はしない。
受信ledgerは上限付きで、world reset/reconfigurationで消去する。

基本装備は新しい装備選択APIを作らず、既存の`transfer_inventory(InventorySource::Player, slot)`を使う。
canonical player slot 5〜8はhead/chest/legs/feet、hotbarは36〜44、offhandは45。
native QUICK_MOVEの装備条件・移動順序を使い、予測と実slot receiptを分離する。
どのitemを装備するかは利用側の判断で、送信のみを装備完了とは扱わない。

## A2の代表シナリオ

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario equipment-entity --accept-eula
```

公式vanillaを一つずつ起動し、各版のSurvival／Creativeで同じconsumerを使う。
一つの接続のまま、hotbar 2のiron bootsをfeetへ転送し、実slot 8受信とRCONの装備を照合する。
empty handで一回攻撃したsheepのHP減少をRCONで確認し、fixtureによる削除の実受信後には
古い対象への要求がentity frameを送らないことをproxyで確認する。続いてvillagerへ一回操作し、
共通APIで実merchant OPENを受信してから切断する。

fixtureは無AIのentityと通常itemを準備する。直前の試験で死んだentityがdeath animation中に残る場合が
あるため、consumerは固定座標・typeに合う最新のspawnを選ぶ。この選択は試験側の規則で、Clientの戦術ではない。
通常操作後の結果は上書きしない。削除fixtureはstale target拒否のためだけに使う。
merchantのlayout／取引、一般entityの現在のmotion・pose・metadata・healthは、残る共通化の対象である。
基本シナリオが通ったことをそれらの全統合完了とは扱わない。

成功runは`trial-1.16.1-2617c684`と`trial-1.21.11-c41dad1a`。同じsource/data/binaryで実行し、
両modeの結果、両JVMのexit code 0、proxyのエラーなしを確認した。
全体テストは単体681件（実サーバー等を要する8件は通常どおりignored）、公開API2件、doctest24件が成功。
ID再利用・同一UUID再spawn・別world/接続・mode不一致・キャンセルの回帰も含む。

## 位置指定interactionの検証

`data/client_api/entity_interact_at_packets.json`は、変更していない公式1.21.11の
`ServerboundInteractPacket`を構築し、write/read/writeで往復した96件を保持する。
VarInt境界、両handとsecondary action、負座標・符号付きzero・有限float境界のwire一致を検査する。
生成元hashと公式JAR/classpath hashは`entity_interact_at_source.json`に記録する。
constructed spawn/destroy packet試験は、旧spawn、同一ID/UUID再利用、別接続/world、mode変更、
非有限座標、送信権限、writer待ち取消と部分frame取消による閉鎖・再送拒否を検査する。
公式codecのfixtureはwireの根拠であり、これらの寿命・取消の根拠とは区別する。

```bash
python3 -B scripts/export_entity_interact_at.py --jar /path/to/server.jar \
  --mappings /path/to/server-mappings.txt --modern-classpath-file /path/to/classpath.txt \
  --work /tmp/entity-at-codec --check
cargo build --locked --example climbing_control_probe -j2
python3 -B scripts/run_entity_interact_at_live.py --binary target/debug/examples/climbing_control_probe \
  --jar /path/to/1.21.11/server.jar --work /tmp/entity-at-live --accept-eula
```

live driverは公式JARを一時ディレクトリへコピーし、loopback上の別portで起動・停止する。
Survival/Creative・両handそれぞれで、空手から同じarmor standの相対Y=0.1/1.8を指定し、
boots/helmetの取得差を元INTERACT_AT frameと独立RCON読取で照合する。
mode不一致と削除済み対象はframeを送らず拒否する。特定のfixtureの結果であり、一般のreachや成功保証ではない。

2026-10-23の公式1.21.11で8ケースが成功した。元packet、独立RCON結果、SDK sourceとbinaryのhash、
公式JARのSHA-1は[evidence](evidence/entity-interact-at-20261023.json)に保持する。
初回はoffhandの結果を`Inventory`だけで読んでtimeoutしたため、fixtureを`equipment.offhand`も読む形へ修正した。
修正後の8ケースではtrace errorなし。SDK実装の変更は不要で、起動したprobe/JVMは停止・一時serverは削除済み。
