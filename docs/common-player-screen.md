# close後の共通プレイヤー画面操作

1.16.1と1.21.11では、完全送信したempty-cursor closeの後も、同じ
`survival.swap_hotbar(main_slot, hotbar)` / `creative.swap_hotbar(...)`で通常在庫を交換できる。
closeのechoがなくても操作を再開できる一方、画面ID・cursor・revision・slotを受信済みと捏造しない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn resume(client: &Client) -> Result<()> {
let ops = client.survival();
let screen = client.screen_state().await?.screen.expect("open screen");
let closed = ops.close_container(screen.id).await?;
let player = client.player_state().await?;
assert!(matches!(player.inventory.player_screen,
    Some(PlayerScreenAccess::SubmittedClose { close }) if close == closed.id));
let swap = ops.swap_hotbar(9, 0).await?;
// 両destinationの新しい実受信をinventory_swap_recordで確認する。再送しない。
# let _ = swap;
# Ok(())
# }
```

`InventoryObservation::player_screen`と`ScreenObservation::player_screen`は、同じcapture境界で
プレイヤー画面の操作状態と根拠を返す。

| player_screen | 意味 |
| --- | --- |
| `None` | 現在のforeign screen、画面状態の欠測、未送信close、world変更など。player画面の根拠にしない |
| `Some(Received)` | native受信でplayer screen 0が確立された |
| `Some(SubmittedClose { close })` | 同じ接続/world/元openingへのcloseをこのClientが完全送信した。server closureの実応答ではない |

`window_id` / `active_window`は最後の実受信のまま保持する。closeを送っただけでは0へ変更しない。
`screen`も実OPEN由来の履歴を保持する場合があり、local UIの判定には`player_screen`とclose recordを使う。
serialized history、未送信intent、失敗・取消・world変更から新しい操作能力を再構成しない。
新しいOPENを受信すれば以前のlocal closeによるplayer画面状態を失効させる。
pending player交換中の再OPEN・mode/cursor変更等はinspectionとして保持し、値の復元で解除しない。

通常在庫のsource/hotbarは実受信したcanonical slotから取り、missing値や古い互換cacheで補完しない。
constructor確認済みcontainerからのplayer slot受信も実際のInventory参照の対応として使える。
元openingへのstorageクリックやclose再送は引き続き拒否する。
返す`InventorySwapRecord`は従来と同じsource/before-I/O capture/send/two-fresh-destination/取消・切断の契約を保持する。
closeを送ったこと自体で採掘や移動の未解決状態を解除しない。

modernは`InventoryObservation::player_screen_revision`に、player screen **0**から実際に受信したrevisionとordinalを別に保持する。
外国menuのrevisionではない。OPENで失わず、world reset/reconfigurationでは破棄する。
close後のplayer SWAPにはこの実revisionを使う。欠測ならI/O前に拒否し、0やforeign revisionで埋めない。
nativeのstale revisionはクリック実行前のlockではなく、実行後のresync契機になるため再送しない。
新しいplayer slot/full packetからplayer revisionを更新しても、foreign `screen_revision`やreceived active windowの値を捏造しない。
legacyにはこのfieldがないので`player_screen_revision == None`のまま、window 0のactionと実応答を照合する。

受信-onlyのmodern native `swap_player_hotbar`/`wait_inventory_swap`は従来の制約を保つ。
close後の共通入口だけが明示的なlocal UI根拠を使い、native pendingの所有権を保持する。
native waitでcommon未解決intentを解除できない。

同じconsumerのfixtureで、survival/creativeのclose後player交換、actual player revision（foreign revisionと異なる値）、
両destinationのfreshness、negative legacy comparison reply、再OPENとrespawn/reconfiguration、missing revision拒否を検証する。
[実サーバー検証](common-client-native-validation.md)では、single chestのclose後にsurvivalでdirt 2をmainからhotbarへ移し、
creativeで元へ戻す。実slot受信と独立RCONのplayer Inventory/chest Items/位置不変を照合する。
共通container open、一般クリック列、非empty cursor close、一般item data、crafting/装備/entity、context/recoveryの統合は引き続き残る。
