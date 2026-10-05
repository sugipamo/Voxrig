# 共通ClientのShift転送

`Survival::transfer_inventory(source, slot)`と`Creative::transfer_inventory(source, slot)`は、
同じ公開型で通常のQUICK_MOVEを1回送る。指定するのは元のsource screen slotで、destinationは
選択版のnative menu規則が決める。handleを選んでも実serverのmodeは変わらない。

```rust,ignore
use voxrig::client::prelude::*;
let intent = client.survival().transfer_inventory(
    InventorySource::Container { screen }, 0,
).await?;
let record = client.survival().inventory_transfer_record().await?;
```

`InventorySource::Player`はnative player screen slot 5..45を受け付ける。
armor 5..8、main 9..35、hotbar 36..44、offhand 45である。crafting/resultは別途対応する。
`Container { screen }`は元の受信済みstorage openingで、storageと付属player slotsを指定する。
数字が同じwindowでも新しいopeningの`ScreenId`は元の操作を引き継がない。
従来の`InventoryClickSource`は同じ型の互換aliasで、通常PICKUPにも`InventorySource`を使える。

実mode、元UI、全transfer対象slotの受信済みbaseline、空cursor、版別item/slot容量と
acceptanceを検査する。modernは実受信revision、legacyは共有action poolの一意なactionを使う。
完全なno-echo close後は、元close/sessionに束縛した明示的なlocal player UIを使う。
未知item、無効count、欠測、未解決data、何も変わらない転送は送信前に拒否する。
通常source/destinationのlegacy NBTとmodern componentsも保持し、受信と同時に保存したregistryで
意味を解決する。容量はdefault値へ戻さず、そのstackの実効値を使う。結合にはnativeのstackable判定と
item種別/dataの同値を要求する。外側の数量差だけを除き、入れ子item/countなどのdataは比較する。
結合先が同じ意味の別表現を持つ場合は、結合先のdataを保持して数量を増やす。

modernは実効`equippable`の追加・変更・削除から装備先を選ぶ。armor slotの受入れは装備先選択と
別に`allowed_entities`を検査し、直接listと実受信のnamed tagを扱う。offhandのnative受入れ規則も
区別する。装備先が埋まっている場合はmain/hotbarへ進み、空でも受入れを拒否するarmorなら
装備を成立したと予測しない。BODY/SADDLEはplayer UIのarmorへ読み替えない。
default以外の装備済みarmorを取り出す規則は追加対応を要する。legacy constructorの`Damage=0`など、
既存のitem別default事実に一致する装備と返却は引き続き対応する。crafting/result/一般装備操作・
非空cursor付きQUICK_MOVEは後続作業。

storageからplayerへの順序は逆順、付属playerからstorageは正順である。
player mainからhotbar、hotbarからmainはそれぞれ正順。nativeが装備先を持つitemは空の対応slotへ
先に移動する。元armor slotからはmain/hotbarへの返却を先に行う。全範囲の既存stackへのmergeを
空slotへの配置より先に行うため、頭のカボチャ1個はmainの空slotより既存hotbarの6個へ合流する。offhandもnative規則に従う。
一度のQUICK_MOVEにnative内部の複数roundが含まれる。カボチャ7個なら頭へ1個、残り6個をhotbarへ
移すため、3つのslotが変化する。client側で同じpacketを繰り返さない。
容量不足なら移せた量を結果に保持し、sourceの残量もfreshな実受信を要求する。
shulker-in-shulkerはnative受入れ条件で拒否する。modern bundleはPICKUP overrideを持つが
QUICK_MOVEをoverrideしないため、通常転送とPICKUPの条件は異なる。

`InventoryTransferRecord`はI/O前の受信済み全slot/cursor、別の`Predicted`結果、実受信結果を保持する。
`changed_slots`のsource/destすべてに送信境界より新しい実packet受信と、数量/native fieldの一致が必要で、
予測を在庫へ書き込まない。source/dest以外の受信slotもnative意味で検査し、configuration/tag所有変更や
未解決dataを競合として保持する。変化しない空cursorは実観測を検査するが、新しいcursor packetは
nativeが送らないことがある。`cursor_inspected`はその元の実受信ordinalを保持し、新規受信と扱わない。

legacyのnative returnは最初のroundで完全に移るならEmpty、partial/internal round後は
元roundのpredecessorになる。独立native oracleでこの値も照合する。異なる実比較値を1回のpacketへ
載せてfull resyncを求め、元window/actionのfreshな実replyと全destinationを待つ。
false comparison replyはgameplay rollbackではない。modernは予測changed mapを送らず、
受信baselineと空cursor hashを使って実changed slotsを要求する。

`inventory_transfer_record()`の`ObservedTransferred`を確認してから次へ進む。
送信取消、world/mode/selection/元openingの変化、unplanned slot/cursorの変更、完了前の復元は
`RequiresInspection`へ保持し、後で値が戻っても解消しない。完了履歴は切断後も保持する。
legacyはactorが一度のwriteを所有する。modernの取消で未完了のwriteが残った場合も
後の似たslot値を成功へ読み替えない。modernのactive state-lock writerの背後からのprompt history取得は
後続の共通context/history整備で対応する。

## Native事実の再生成

未変更の公式1.16.1 / 1.21.11 JARとmappingを使う。modern bundleの全40 classpath entryを
元SHA-256へ照合する。未spawnの最小ServerPlayerに実Inventory/EntityEquipment、実GameMode、
native default flagsを供給し、元`clicked(QUICK_MOVE)`、slot、item、packet codecを直接呼ぶ。
ゲームmethod bodyは置換・転載しない。このprimitive oracleは接続/mode/所有/復旧の証明ではない。

```bash
python3 scripts/export_inventory_transfers.py \
  --downloads "$DOWNLOADS" \
  --modern-classpath-file "$MODERN_CLASSPATH" \
  --runtime-output .local/inventory-transfer-oracle \
  --check
```

JVMは1つずつ、heap512MiB/CPU1で実行する。profilesにdefault itemの974 / 1,504種類の転送先、
装備slotの容量/受入れ/default NBTを保持する。casesは4,988 / 6,238、packet codecは18ずつ。
空・merge・blocked・装備占有、source境界、storage 9種類、armor/offhandと内部複数roundを含む。
要求sourceとnative setter後の実sourceを区別する。Rust予測は全11,226件の実全slot結果と
legacy returnに一致する。source JSONは元配布物・生成器・raw・出力のhashを保持する。
公式JAR、mapping、classやruntimeはpackageへ含めない。

変更したmodern装備先の検査は`export_equipment_transfers.py`で別に再生成する。
引数は上の生成器と同じで、`equipment_transfer_cases-1.21.11.json.gz`と
`equipment_transfer_source.json`へ保存する。全8装備先、追加/変更/削除、許可対象の欠落・空list・
player/cow list・named tag、装備占有、armorへの1個と残量の別slotへの移動、main/hotbar sourceの
648ケースで元のslot受入れと全slot結果を照合する。item/entity tag宣言も同じ元registry lookupから
取得する。このprimitive検査は実接続の受信・mode・取消を証明するものではない。

変更したequippableの実接続検査は`trial-1.21.11-e2341241`で両modeとも成功した。
受信したstone3個にhead装備先とplayer許可を追加し、共通APIの1回の転送でhead1個/hotbar2個に
分ける。3つのchanged slotすべてにfresh実受信を要求し、同じmetadataと数量を照合した。
元serverのRCONもhead/hotbarの数量、equippable、custom-data markerを独立して確認した。
既存の両版nativeシナリオも同じ入力snapshotで成功し、両JVMは正常終了した。
非defaultの装備済みarmorの取り出しや任意item activationの実装完了を意味しない。


共通ClientのShift転送の実接続runは`trial-1.16.1-841caa80` /
`trial-1.21.11-a6428298`で成功した。両runは同一consumer binaryを使い、各版9件の完了recordを
`data/client_api/inventory_transfer_native_evidence.json`へ保持する。チェストの逆順転送、close後の
main/hotbar転送、カボチャの自動装備・既存stackへの合流・hotbarからの再装備、防具の装備と返却を
両modeで実行する。legacyのdefault `Damage=0` NBTも元bytesのまま受信・送信する。
変化した全slotはfreshな実受信、変化しないcursorは元ordinalの実観測を検査する。
RCONはstorage/player数量とhead装備を独立して照合し、位置も不変だった。
旧版は`Inventory`内のSlot 103、modernはnative `equipment.head`を照合するが、共通APIのheadは
両方canonical player slot 5である。両JVMはexit 0、tmpfs runtimeは退避後に削除済み。

検証側の失敗3 runもraw hash・理由・実行時inputとともに保持する。空mainより既存stackへのmergeを
優先するnative規則、modernの独立したhead保存形式、play/RCON別経路での即時fixture cleanupを
修正した。外部cleanup前は実native tick進行を確認するが、tickは処理ACKへ読み替えない。
slot/cursorの実受信条件は緩和しない。保存済みmodern native player dataも装備形式の独立診断に
使用した。native入力のsnapshot後に行ったsource互換名のre-export・diagnostic wordingと生成器commentの
整理は動作を変えず、現在のsourceは別途全テスト・compiler・packageで確認する。
