# 共通のアイテム使用（use_item・release_use_item・use_on_block）

`Client::survival()`と`Client::creative()`の両方に、同じ型で次の操作がある（`Feature::ItemUse`）。

| 操作 | 送るもの | 例 |
| --- | --- | --- |
| `use_item(hand)` | 手に持ったアイテムを対象なしで使う | 食べる・飲む、盾を構える、弓を引く、投げる |
| `use_item_expect(hand, selected_hotbar, item)` | 同じロック内で選択スロットと受信済み item ID を検査してから使う | 呼び出し前の確認後に手持ちが変わった場合は送信せず型付きエラー |
| `release_use_item()` | 使用中のアイテムを離す | 弓を放つ、盾を下ろす、食事を途中でやめる |
| `use_on_block_expect(target, face, cursor, hand, selected_hotbar, item)` | 同じ送信ロック内で選択スロットと指定handの受信済みitem IDを検証してblockを使う | End portal frameへのender eye挿入など |
| `use_on_block(target, face, cursor, hand)` | 手に持ったアイテムでblockの面を使う | 松明・ドア・レールなどの設置、ドア・レバー・ボタン・チェストの操作 |

```rust,no_run
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
survival.select_hotbar(0).await?;
let dispatch = survival.use_item(Hand::Main).await?;        // 送信の完了だけ
// serverが使用を始めたかは受信した状態で確かめる
let using = client.player_state().await?.using_item;      // Some(ObservedValue { value: Some(Hand::Main), .. })
survival.release_use_item().await?;

// 床のblockの上面に松明を置く。cursorは対象cellの中の当たり位置（各軸0..1）
survival.select_hotbar(3).await?;
survival.use_on_block([10, 63, 20], BlockFace::Up, [0.5, 1.0, 0.5], Hand::Main).await?;
client.wait_for_block([10, 64, 20], std::time::Duration::from_secs(3), |b| {
    b.is_some_and(|b| b.name == "minecraft:torch")
}).await?;
# let _ = (dispatch, using);
# Ok(())
# }
```

## 契約

- すべて**送信だけ**の操作で、`DispatchReceipt`は完全な送信を表す。serverの受理や結果ではない。
  結果は受信した状態で確かめる。
  - 使用の開始と終了: `PlayerObservation::using_item`（下記）。
  - 設置・操作の結果: 受信したblockの状態（`wait_for_block`、world capture）。
  - 消費: 受信したinventoryのslot。
  - 開いた画面: `screen_state()`。
- 1.21.11は`DispatchReceipt::interaction_sequence`に、使った操作番号を返す（`use_item`と`use_on_block`）。
  1.16.1のprotocolには操作番号がないので`None`。
- 送信前に、受信したgame modeがhandleのmodeと一致することと、既存の未解決操作（設置・採掘・
  inventory操作・container・乗り物・飛行など）がないことを検査する。自動の再送はしない。
- `use_on_block`は明示したpos/face/cursor/handを送り、視線・接地・固定4.5の検査はしない。
  `cursor`は有限で各軸0..1、座標はwire範囲内であることを要求する。
  距離は公式serverの条件（1.16.1は足元→block中心の距離二乗64未満、1.21.11は目→block AABBが
  interaction-range属性＋1未満）を使う。未受信のmodern姿勢・属性はserver判定に委ねる。
  手に持ったものとblockの組み合わせで何が起きるか（設置か操作か）はserverが決める。
  - 両版のserverは処理のあとに、対象cellと面の隣のcellのblock更新を送り返す
    （1.21.11はreach外・当たり位置がcellの外の場合を除く）。
  - しゃがみ状態は今のまま使う。しゃがんでチェストに設置したい場合などは、利用側で先にしゃがむ。
- `use_item`は今の向きを使う。1.21.11はpacketに向きを含み、serverはplayerをその向きにそろえる。
- `release_use_item`は公式clientと同じく、位置0・面DOWN（1.21.11は操作番号0）の
  RELEASE_USE_ITEMを送る。使用中でなくても送れる（serverは何もしない）。
- 1.21.11の版固有APIの`use_on_block`もSurvivalで明示送信できる。
  共通APIの`Survival::use_on_block`と同じく、材料の消費や設置を確認しない。
  1個のfull cubeを置いて対象と材料の受信まで確認したい場合は、`Survival::place_cube`を使う。

## 使用中の受信状態（`using_item`）

`PlayerObservation::using_item`は、serverから受信した自分のLivingEntityのflags
（1.16.1はmetadata index 7、1.21.11は8。bit 1が使用中、bit 2がoff hand）。

- `None`: このworldでflagsをまだ受信していない。1.21.11は最初の使用まで送られないことが多い。
  1.16.1はjoin時に送られる。
- `Some(ObservedValue { value: Some(hand), .. })`: serverが`hand`のアイテムを使用中と報告した。
- `Some(ObservedValue { value: None, .. })`: serverが使用していないと報告した。
- 送った`use_item`をここに先回りして書くことはしない（受信だけ）。respawnやworldの変化で`None`に戻る。

## 継続操作との関係

継続操作（`start_control`）の実行中も`use_item`・`release_use_item`・`use_on_block`を送れる。
アイテム使用中の減速は[共有物理エンジン](physics-engine.md)が公式clientと同じ規則で再現する
（[比較基準](movement-oracle.md)の場面`use_*`で両版ともbit単位で一致）。

- 入力の縮小: 1.16.1は入力を×0.2（しゃがみの後）。1.21.11は×0.98の後に、使用中のアイテムの
  `minecraft:use_effects`の`speed_multiplier`を掛ける（既定0.2。槍は1.0）。
- ダッシュ: 使用中は開始しない（1.21.11は`can_sprint`がfalseのとき。槍は開始できる）。
  すでにダッシュ中なら、使用を始めても止まらない（公式clientと同じ）。
- 使用中かどうかは**受信した**`using_item`に従う。公式clientは使用を自分で始めた時点から減速するが、
  Voxrigはserverの報告を受け取った時点から減速する（往復の遅れの分だけ、最初の数tickは減速しない）。
  - `release_use_item`を送ると、その時点で減速をやめる（公式clientと同じ）。古い「使用中」の受信は使わない。
  - 使用中のアイテムが受信したinventoryから分からない場合（1.21.11で`use_effects`が決まらない）は、
    sessionを`Paused`にして何も送らない。
- `ControlFrame::using_item`に、そのtickに適用した減速（`ItemUse`）が入る。

## 実サーバーでの確認（2026-10-08）

公式`server.jar`（1.16.1・1.21.11）をoffline-mode・peacefulでlocalhostに起動し、
`examples/item_use_probe.rs`で確認した（consoleで場所とアイテムを用意する）。
どちらの版も次がすべて期待どおりで、serverのlogに警告はなかった。

| 確認 | 1.16.1 | 1.21.11 |
| --- | --- | --- |
| 金のリンゴを食べる（main hand） | 使用中の受信 65 ms後、終了と個数2→1の受信 1.61 s後 | 21 ms後、1.60 s後 |
| 盾（off hand）を構えて下ろす | 使用中→離して44 ms後に非使用 | 同じ（43 ms） |
| 弓を1.2 s引いて放つ | 矢4→3 | 矢4→3 |
| 盾を構えたまま継続操作でダッシュキーを押して歩く→下ろす→歩きながら構える | 減速・ダッシュなし→ダッシュ開始→ダッシュ継続のまま減速。serverの補正0回、serverの最後の位置が予測とbit単位で一致 | 同じ |
| 床の上面へ松明 | 松明が設置され4→3 | 同じ（操作番号4） |
| 素手でレバー・ドア | `powered=true`・`open=true`を受信 | 同じ |
| 4.5より遠い対象（この旧検証時の挙動。step 3で変更） | 送信前に拒否 | 送信前に拒否 |

この確認で、1.21.11の効果packetのflagsにblend（8）があることが分かった。以前は受信時に拒否して接続を
止めていた（金のリンゴの効果で発生）。公式のpacket定義に合わせて受け入れるように直した。


## 明示hitの到達距離（拒否削減 step 3）

固定4.5と立位eyeによる旧制限を外し、公式serverの距離条件に変更しました。
`placement_check` は従来のclient pick range/default boxの任意診断です。
厳密な`place_cube`/`open_container`はreceipt用に残し、移動・空中・非first-hitの設置は
bodyの`place_cube`/`PlaceBlock`、容器操作は`ActivateBlock`を使えます。
根拠・期限・Acceptedと実際の効果の区別は
[明示block操作](explicit-block-interactions.md)を参照してください。採掘はこの変更の対象外です。

### Atomic held-item expectations

Both Java 1.21.11 and 1.16.1 adapters provide the additive common API:

```rust,no_run
# async fn example(client: &voxrig::Client) -> voxrig::Result<()> {
let food = client.registry().item("minecraft:bread")?.id;
let sent = client.survival()
    .use_item_expect(voxrig::Hand::Main, 0, food).await?;
// `sent` proves complete dispatch only. Observe received state for the result.
# let _ = sent;
# Ok(()) }
```

`Creative::use_item_expect(hand, selected_hotbar: u8, item: RegistryId)` has the
same signature and returns `Result<DispatchReceipt>`. The native Java 1.16.1
`Bot::use_item_expect` returns `Result<()>`, like its existing `use_item`.
The original `use_item` APIs keep their behavior.

The expected hotbar index is 0..8. Under the native coherent state lock
(`coherent_state_gate` on 1.16.1; session state mutex on 1.21.11), the operation
checks both the current selection and the held item's version-bound registry ID,
then keeps that lock through complete packet dispatch. Main hand reads player
inventory slot `36 + selected_hotbar`; offhand reads slot 45 and **also checks the
expected hotbar selection**. A selection received from the server or fully
submitted locally is usable. The item slot must have received provenance;
local cache predictions are insufficient. Native 1.16.1 `select_hotbar` clears
its prior selection identity before sending and records `Submitted` only after
complete dispatch. A disagreement between native compatibility selection and the
common selection is treated as unknown. Unknown, unavailable or empty slots
refuse use. Count, NBT and components are deliberately not part of this item-ID
contract: a same-ID replacement is allowed, even if its data differ.

A state mismatch or missing identity sends no use-item packet and returns
`ErrorKind::State`. `Error::use_item_refusal()` returns `UseItemRefusal` with the
hand, expected/observed selection and expected item ID/observed received slot.
`observed_item == None` means no received slot value; explicit empty and
unavailable values remain distinct. Invalid indices, foreign-version IDs and
non-item registry IDs return `InvalidInput`. Other admission/connection failures
retain their existing errors. This checks client state at dispatch, not a future
server state, and neither dispatch nor identity matching proves that use began
or that food was consumed. Check received metadata, health and inventory updates.

`voxrig::body::HeadlessBody::use_item_expect(hand, selected_hotbar, item)` returns
`Result<ActionId>` on both versions, without extending the existing exhaustive
public action enums. It uses the same atomic validation and
world fence. The asynchronous action is Rejected on identity refusal;
`body.backend().use_item_refusal(action_id)` retains the typed details with the
normal action retention policy. Its Accepted status has the Body API's usual
meaning; it is not a consumption receipt. Use `body.backend().use_item_expect(...)` to submit through a `Body` facade.
The existing capability list and unchecked actions retain their behavior.

Local official-server evidence for both native versions is retained in
[the 2026-10-25 live summary](evidence/use-item-expect-live-20261025.json), with
compressed full reports and source/binary hashes alongside it. Each version
checks Survival and HeadlessBody, both hands, actual bread consumption, and
slot/item mismatches with zero use-item packets.

### Atomic block-use expectations (`use_on_block_expect`)

共通 `Survival` / `Creative` の両方に、1.21.11 / 1.16.1 nativeで次の追加APIがあります。
型は `voxrig::client`（`RegistryId` は `voxrig::client::registry`）からimportできます。

```rust,ignore
pub async fn use_on_block_expect(
    &self,
    target: [i32; 3],
    face: BlockFace,
    cursor: [f32; 3],
    hand: Hand,
    selected_hotbar: u8,
    item: RegistryId,
) -> Result<DispatchReceipt>;
```

```rust,no_run
# async fn example(client: &voxrig::Client) -> voxrig::Result<()> {
let eye = client.registry().item("minecraft:ender_eye")?.id;
let sent = client.survival().use_on_block_expect(
    [0, 65, 2], voxrig::BlockFace::Up, [0.5, 1.0, 0.5],
    voxrig::Hand::Main, 0, eye,
).await?;
# let _ = sent;
# Ok(()) }
```

`player_state()` の確認から呼出しまでにselectionやinventoryが更新されても、送信と同じ
ロック内（1.16.1: `coherent_state_gate`、1.21.11: session state mutex）で再検証します。
一致の確認からpacketの完全送信までロックを保持します。期待値の型と意味は上記
`use_item_expect(hand, selected_hotbar: u8, item: RegistryId)` と同じです。
main handは現在の選択スロットのplayer inventory `36 + selected_hotbar`、offhandは45を読み、
**offhandでも選択スロットを検証**します。受信済みitem IDと、受信済みまたは完全送信済みの
selectionを要求し、count・NBT・componentsは比較しません。同じIDへの置換は許可します。

不一致・未知・未受信・空・unavailableはblock-use packetを送らず、`ErrorKind::State` と
既存の `UseItemRefusal` を返します。`Error::use_item_refusal()` からhand、
`expected_selected_hotbar` / `observed_selected_hotbar`、`expected_item` /
`observed_item` を参照できます。未受信の観測値は `None` です。
不正slot（0..8以外）、別versionまたはitem以外の `RegistryId` は `InvalidInput` です。
その他のadmission条件、距離・cursor・wire座標、packet内容は `use_on_block` と同じです。
1.21.11のreceiptはinteraction sequenceを含み、1.16.1は `None` です。
成功は完全送信だけを示します。End portal frameの `eye=true` やinventoryの消費など、
serverの結果は受信状態で確認してください。serverが将来扱う手持ちの保証ではありません。
既存 `use_on_block` と版固有APIの挙動は変更しません。

bodyの `ActivateBlock` / `PlaceBlock` は専用のworld fence・証拠境界・受理判定を通り、
`use_item_expect` のone-shot送信経路と異なります。期待itemの追加はその別経路に契約を
通す必要があるため、body側のblock-use expectationはfollow-upです。

2026-10-25に公式ローカルserver両版で、Survivalのmain/off handからEnd portal frameへ
ender eyeを挿入し、serverの `eye=true` と受信inventoryの3→2を確認しました。
slot不一致・item不一致は各0 block-use packets、一致は各1 packetです。
[実機サマリー](evidence/use-on-block-expect-live-20261025.json)と隣接の圧縮reportに
元packet・期待値/観測値・source/binary hashを保持しています。

See [guarded native admission](guarded-item-use.md) for additive synchronous checks
after readiness/writer waits and explicit received-empty UseOn expectations.
