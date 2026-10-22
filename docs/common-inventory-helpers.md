# inventory の補助（`move_inventory`・`compact_inventory`・`fill_crafting_grid`・`craft_once`・`craft_once_checked`）

共通の `Survival::click_inventory`（通常の左・右クリック）と `transfer_crafting_result` を組み合わせた補助です。両版とも同じ手順で動きます。各クリックは、その記録が `Pending` を抜ける（受信した結果がそろう）まで最大 3 秒待ってから次へ進みます。`ObservedClicked` 以外で終わったクリックはエラーになり、そこで止まります。

## `move_inventory(&mut record)`

source UIと移動元・移動先のnative slot番号を明示します。装備の自動選択はしません。
`Survival`と`Creative`の両handleで使え、実受信modeを検査します。

```rust,no_run
use voxrig::client::prelude::*;
# async fn equip(client: &Client) -> Result<()> {
let mut record = InventoryMoveRecord::new(InventorySource::Player, 9, 5);
let result = client.survival().move_inventory(&mut record).await;
println!("{:?}: {} confirmed clicks", record.stage(), record.completed_steps());
// エラーでもrecordとcursorを調べてから次の操作を決める。
result?;
# Ok(())
# }
```

空の実受信cursorと両slotの受信値から、左PICKUPの「元を拾う→先へ置く／交換→
残量・交換されたstackを元へ戻す」を最大3回で組み立てます。空slotへの移動と全量mergeは
2回、部分mergeと占有済み装備などの交換は3回です。player armor 5..8・offhand 45の
受入れ・容量・束縛は既存の`slot_policy`を共用し、data付き装備をdefaultへ戻しません。
戻り先と炉の燃料tagを含めて事前検査し、3回でcursorが空にならない組合せや無変化、
未受信・非対応の燃料tagは元slotを拾う前に拒否します。
同じUI内の通常PICKUP対応slotが対象です。異なるscreen間の移動やresult取得は含みません。

各子クリックの完全送信とfreshなslot/cursor受信（legacyは取引応答も）がそろい、
`ObservedClicked`になってから次へ進みます。`record.clicks()`は子ID・送信前状態・予測・
実受信結果を順に保持し、`completed_steps()`は確認済みの段階数です。全手順の確認後だけ
`ObservedMoved`になります。予測を実受信在庫へ書き込みません。

recordは呼出側が保持します。futureをdropしても既に保持した子と完了段階は残り、
親は`Stopped`になります。各回の3秒timeout、送信不確実性、前提・screenの変更でも止まります。
rollback・再送・別slotへの迂回は行わず、cursorにstackが残る場合があります。
`Stopped`のrecordを再実行できません。期限後にserverが適用する場合もあるため、
新しい操作の前に受信在庫と子recordを照合してください。

同じclientの在庫操作は、このhelperと必要なrefreshが終わるまで呼出側で直列化します。
各子は既存clickのledger・排他・接続/world/screen寿命を使いますが、helper全体を単一の
server transactionとして扱いません。送信futureの取消で子IDが戻る前に止まった場合は、
`active_slot()`と`refresh_inventory_move(&mut record)`で元の子を照合します。
refreshは同じsession・mode・UI・slot・次のattemptだけを読み取り、送信も再開もしません。
adapterは直近の子recordを保持するため、次のクリックを認可する前にrefreshしてください。
完了済みの子履歴は後の現在状態で書き換えず、遅い受信で子の確認が増えても停止した親は
`Stopped`のままです。

両版のconstructed packet fixtureで、2回／3回の移動、部分merge、占有済み鎧・offhand交換、
装備data保持、束縛・不適合拒否、2回目／3回目の競合、timeoutと遅い受信、screen変更、
途中取消後のcursorと親・子recordを検査します。
`examples/climbing_control_probe.rs`と`scripts/run_exact_inventory_move_live.py`は公式1.21.11の
占有済み鎧・offhand交換、束縛拒否、取消後のcursorを、元packetと独立したRCONで検証します。

## `compact_inventory()`

1. cursor に持っている stack を収納します。同じ item と data で空きのある stack を先に、なければ最初の空き slot を使います。
2. 収納の末尾から先頭へ（main 9..35、次に hotbar 36..44 の順）見ていきます。前方に同じ item・data で空きのある stack があれば、拾って前方へ詰めます。

player 画面でも、開いている container の player 部分（検証済みの layout がある場合）でも動きます。local の `close_container` 完全送信後は、受信した旧 screen が残っていても player 在庫を使います。戻り値は全クリックの記録です。

## `fill_crafting_grid(ingredients)`

開いている crafting grid（player の 2×2、または作業台の 3×3）に手で材料を並べます。

- `ingredients` は行優先で、grid の大きさと同じ長さにします。`Some("oak_planks")` は `minecraft:` を補います。
- まず grid に残っている物を収納へ戻します。
- 各材料について、収納の stack を左クリックで拾い、grid に右クリックで 1 個置き、残りを元の slot に戻します。
- 最後に、受信した結果 slot を最大 2 秒待ちます。戻り値は `CraftingFillRecord { clicks, result }` です。

## `craft_once(ingredients)`

`fill_crafting_grid` の後、結果を `transfer_crafting_result`（shift で収納へ）で 1 回取り、その記録の結果を最大 3 秒待ちます。fill 後に結果 slot が出なければエラーです（材料は grid に残り、次の fill で戻ります）。戻り値は `CraftOnceRecord { fill, take }` で、take が `Pending` や `RequiresInspection` でも `Ok` を返します。

## `craft_once_checked(ingredients)`

同じ手順・期限で実行し、take が `ObservedTransferred` の場合だけ `Ok` を返します。期限時点で `Pending` なら `CraftOnceError::OutcomeUnknown { record, cause: CraftOnceUnknownCause::Timeout }`、inspection が必要なら cause は `RequiresInspection` です。`record.take.id` と最後の `record.take.stage` を保持します。入力不正・fill のエラー・結果なし、および take の frame write 前に予約・所有権・前提状態の検査で拒否された場合は、元のエラーを保持した `CraftOnceError::Failed` です。

take が受理・保持された後、その送信が失敗して配送が不明になった場合は、`CraftOnceError::OutcomeUnknown { record, cause: CraftOnceUnknownCause::RequiresInspection }` を返します。fill 記録と保持された take の記録（`record.take.id` を含む）を渡し、再送しません。take が保持されている、またはその記録が `RequiresInspection` になったという事実だけでは配送不明と判定しません。送信前の拒否は `Failed` のままです。公開型・シグネチャと `craft_once` の動作は変更しません。

期限超過は失敗・取消の証明ではなく、server が後から適用する場合があります。受信在庫や `crafting_take_record()` で再観測・照合してから判断し、無条件に再送しません。

## 制限

- 同じ item で data（enchant・名前など）が違う stack はまとめません。
- 材料は item 名で探します。data は区別しません。
- recipe book（`place_recipe`）は使いません。サーバーの判定は受信した結果 slot で見ます。

## 公式サーバーでの確認

`examples/inventory_helpers_probe.rs` で両版を確認しました。

- 散らばった板材（5・7・60）と丸石（3・2）を、それぞれ 64+8 の 2 slot と 5 の 1 slot にまとめました（5 クリック）。
- player の 2×2 で棒 4 本を作りました（板材 −2）。
- 作業台の 3×3 で木のツルハシを作りました（板材 −3、棒 −2）。

明示slot移動helperの実接続は2026-10-22の公式1.21.11で7ケース成功しました。
両modeの占有済み鎧・offhand交換、Survivalでの束縛拒否（クリック0回）、Creativeでの束縛鎧交換、
1回目確認後のhelper取消によるcursor残留を照合しています。取消後の返却は観測後に呼出側が
明示した別クリックで、helperのrollbackではありません。Damage=7とcustom-data markerを
実slot/cursor受信およびRCONの装備・在庫で確認しました。元serverは正常終了しています。
[実接続の記録とhash](evidence/common-exact-inventory-move-20261022.json)に、子ID・完了段階、
元packet、独立RCON、公式JAR・consumer binary・source・raw artifactのhashを保持します。
