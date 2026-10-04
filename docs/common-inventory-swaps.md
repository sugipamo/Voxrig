# 共通Clientの在庫交換

`Client::survival().swap_hotbar(main_slot, hotbar)`と`Client::creative().swap_hotbar(...)`は、
通常在庫のscreen slot 9..35とhotbar index 0..8のwhole stack交換を一度だけ送る。
1.16.1と1.21.11で同じ`client::inventory::InventorySwapRecord`を返し、利用側の版分岐を不要にする。
handleのmodeは送信時に実際の受信modeと照合する。creativeからの交換も通常のクリックで、itemを生成しない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn exchange(client: &Client) -> Result<()> {
let survival = client.survival();
let attempt = survival.swap_hotbar(9, 0).await?;
// Pendingでも同じswapを再送しない。続く読出しはpacketを送らない。
let latest = survival.inventory_swap_record().await?;
# let _ = (attempt, latest);
# Ok(())
# }
```

これは在庫・container統合段階の最初の操作である。一般containerのクリック列、split/shift click、
crafting、装備、一般NBT/components付きstackの操作は後続作業に残る。
`Feature::InventorySwap`はplayer交換、`Feature::Containers`は既に開いたstorage/hotbar交換とopening-bound closeのRestricted。一般open/クリック列は残る。
既存のmodern専用`swap_player_hotbar` / `wait_inventory_swap`とlegacy Bot APIも維持する。

## 開いたstorageとの交換

`survival.swap_container_hotbar(screen_id, source_slot, hotbar)`と同じCreative handleのmethodは、
実際に開いたconstructor確認済みstorageとhotbarのwhole stackを通常のSWAPで交換する。
`Client::screen_state()`の現在の`ScreenId`を渡す。packet長からplayer slot位置を推定しない。
source_slotはstorage側のみで、appended player slotは拒否する。hotbarはindex 0..8。
両slotが非空の場合と一方が空の場合を扱い、default dataとempty cursorを要求する。

`InventorySwapSource::PlayerMain`はplayer screen、`Container { screen }`はその接続/world/OPENの画面を表す。
`source_slot: u16`と`hotbar_screen_slot: u16`は実クリック画面でのindex、`hotbar: u8`はcanonical hotbar index。
`initial_screen`はI/O前の実container captureで、player交換ではNone。両方とも同じrecord・stage・読出しAPIを使う。

受信full contents、constructor layout、現在の画面identity、playerとのhotbar受信対応が揃っていることを要求する。
画面のclose・再OPEN・world変更は元attemptのinspectionとして残り、同じnumeric ID/期待値の復元で解除しない。
constructor確認済みの9 storage menuとplayer main slotのnative受入れ条件を送信前に検査する。
shulker boxへのshulker box収納は`InvalidInput`で拒否し、click/owner/未解決recordを作らない。
元のsourceが空でない場合は取り出し許可、非空hotbarはslotの全量受入れ許可と容量を要求する。
実serverの追加制限や操作許可を保証する契約ではなく、期待する両destinationが実受信されない場合は未解決のまま保持する。
特殊UI/slot・一般click列は後続段階で拡張する。[native通常クリック調査](common-inventory-clicks.md)を参照。
未解決のcontainer交換とplayer交換は同じcommon ownerを使い、一方を別の入口から迂回しない。
未解決のcloseも同じadmissionで拒否し、close送信を保持した元のScreenIdにはクリックしない。
closeのactual receiptとdispatchの区別は[共通container close](common-container-close.md)を参照。

legacyは実window IDの番号poolからactionを確保し、同じwindow/actionの実比較応答を照合する。
modernは実container revisionを送る。どちらもclicked slot/hotbar mappingはそのopeningから確定し、
取消で別の画面へ再送したり、保存したScreenIdを新接続へ再構成したりしない。
完了したhistorical recordは凍結し、次の明示的交換は新しいcurrent captureから検証する。

## 前提と記録

実受信済みplayer screen 0、空cursor、両slotの完全な受信値、handleに一致するsurvival/creative modeと、
未解決dispatchがないことを要求する。slotの欠測やLocalCacheをEmpty/受信値へ補完しない。
player画面は、同じopeningへのcloseを完全送信した明示的な`SubmittedClose`からも操作できる。
actual window/cursor/slotを送信から生成せず、modernは別に保持したactual player-screen-zero revisionを使う。
根拠・world変更・missing revisionの扱いは[共通プレイヤー画面](common-player-screen.md)を参照。
材料名・版付きregistry ID・count・default dataを検査する。default stack以外は現在の共通入口では未対応。
modernはnative decoderの未対応components flagや画面revisionの欠測も拒否する。
中身が同じ2slotは変更packetが返らない場合があるため、不要な交換としてI/O前に拒否する。

`initial`、`source_before`、`hotbar_before`をadapterの同じ境界でI/O前に保持する。
`InventorySwapId`は元接続/world/attemptに結び付いたopaqueな識別子で、Deserializeを持たない。
保存したJSONは診断であり、新しい接続の操作能力にはならない。
`send.after_sequence`は送信前の実受信境界、`dispatched`は完全なframeが書かれたことを示す。
送信からslot予測、画面revision、受信ordinal、cursorの結果を作らない。

| native field | 1.16.1 | 1.21.11 |
| --- | --- | --- |
| `send.legacy_action` | actorが既存クリックと同じpoolから確保するtransaction番号 | `None` |
| `send.screen_revision` | `None` | 実際に受信したplayer-screen revision |
| `legacy_reply` | 同じwindow/actionの新しい比較応答。falseはclickのrollbackではなくresyncを表す | `None` |

legacyは0x09へwindow 0、main slot、hotbar button、short action、SWAP、受信済みの非空prestackを比較値として送る。
公式handlerはクリックを先に実行してからreturned stackと比較する。SWAPのnative returnはEmptyなので、
prestackとの不一致はnegative比較応答と実在庫のfull resyncを発生させる。正しいEmptyを送る場合は
serverがslot更新を抑止し、実受信根拠を得られない。共通入口は予測slotを受信値と扱う代わりにこのresyncを要求する。
`send.legacy_comparison`に送ったstackをI/O前に保持する。再送や追加クリックは行わず、
negative応答への必須protocol replyだけを返す。negative応答単独では完了にもrollbackにもならない。
公式codecで形式とpacket IDを確認している。
modernは既存native SWAP形式と空modified-hash map/空cursor hashを使い、予測hashで実更新を抑止しない。
modernのstale revisionはクリックの実行を防がないため、revisionをretry用のlockと扱わない。

## 結果・取消・履歴

| stage | 意味 |
| --- | --- |
| `Pending` | 必要な新しい受信が揃っていない。再送しない |
| `ObservedSwapped` | 完全送信と両方の新しいexact destinationを照合。legacyは同じtransactionの実比較応答も要求 |
| `RequiresInspection` | 最初の不整合、session/mode/cursor/screen変更、送信不確実性等を保持 |

`source_receipt`は元hotbar stack、`hotbar_receipt`は元main stackと完全一致し、
各slotの実更新ordinalが`send.after_sequence`より新しいことを要求する。
片側の更新、cache revision、無関係なslot更新、比較応答だけでは完了しない。
先に届いたdestinationが完了前にprestackへ戻っても、あとで期待値が揃っても競合を消さない。
cursor・画面・modeの一時的な変更もpacket適用時に保持し、復元で解消しない。
この照合はclient receive evidenceであり、変化のactorやitemの由来の証明ではない。

未解決の間は二重交換、held-slot選択、block/motion等の通常操作を拒否する。
legacyはactorが通常dispatchを排他し、controlがidleかつ他transactionがない状態から開始する。
autonomous physicsも競合を避けるため一時停止し、matching destinationと同じactionの比較応答を揃えて
`inventory_swap_record()`で同じattemptのgateを解放する。解放packetは送らない。
legacyの番号はcommonと既存クリックで共有し、共通交換では枯渇時に拒否して同じ番号を循環利用しない。

legacyではwaiterを取消してもconnection-owned taskが同じクリックを一度だけ送る。
modernではwriter取得前に取消したintentは未送信のまま残り、その後に他の原因でslotが一致しても
交換成功にしない。partial writeと終了は不確実性を保持する。自動retryや補償交換はない。
既存modern専用のwait APIからcommon未解決intentを解除することも拒否する。

完了後は次の新しい交換を明示的に開始でき、最新のmode/screen/cursor/両stackを再検証する。
完了した両destinationは履歴として保持し、現在のinventoryと分ける。
新しいcommon attemptを開始するまで直前のcommon recordを保持し、閉じた接続でも読める。
取消後の待機再開は同じrecordの読出しで行い、未解決状態を引き継いだ新しいクリックにはしない。

## 検証

同じconsumerを両版のnative packet fixtureへ通し、before-I/O capture、wire format、両slotのfreshness、
legacyのmatching応答、raw Inventory番号のhotbar変換、途中の競合、取消、完了後の次の交換を検査する。
actorの所有権をmotion/placementと混同せず、write失敗で不確実な接続を再利用しないことも検査する。

`scripts/VerifyLegacyInventorySwap.java`は未改変の公式1.16.1 server JARのpacket codecとnative registryで、
`data/client_api/legacy_inventory_swap_packets.json`のplayer 3・storage 2の計5ケースをdecode/encodeし、packet ID 0x09とcomparison stackを含むpayloadを確認する。
Java 21とSHA-1検証済みJARを使い、ログを残す作業directoryから次の形で実行できる。
server/worldやネットワークは起動しない。MinecraftのJAR・mappings・bytecodeはpackageへ含めない。

```bash
java -Xmx1024M -XX:ActiveProcessorCount=1 --class-path /absolute/path/1.16.1-server.jar \
  /absolute/path/Voxrig/scripts/VerifyLegacyInventorySwap.java \
  /absolute/path/Voxrig/data/client_api/legacy_inventory_swap_packets.json \
  /absolute/path/results.json
```

公式vanilla両版ではさらに新しい接続で、survivalのmain stone 3/hotbar dirt 2を交換し、
続いて受信modeをcreativeへ変更してmain dirt 2を空hotbarへ移す。
両方の実受信を待ち、独立したRCONでslot・item・個数と位置不変を確認する。
再実行・実行結果は[共通native検証](common-client-native-validation.md)を参照。

同じconsumer/native試験ではさらに開いたsingle chestのstone 7をsurvivalで空hotbarへ取り出し、
同じopeningでcreativeへmodeを変更して戻す。両destinationのfresh receipt、元opening、
独立RCONのcontainer Items/player Inventory/位置不変と正常終了を照合する。
