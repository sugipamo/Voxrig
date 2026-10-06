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
arm swing、retry、position-specific INTERACT_AT、health/metadataの正規化は供給しない。
返る`DispatchReceipt`は完全送信だけを表し、damageや画面表示、取引の成功を保証しない。
別途実受信またはサーバー側の結果を確認する。画面OPEN自体にもentityへの因果ACKはない。

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
