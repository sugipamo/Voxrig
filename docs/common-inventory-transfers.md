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

`InventorySource::Player`はnative player screen slot 1..45を受け付ける。
crafting入力1..4、armor 5..8、main 9..35、hotbar 36..44、offhand 45である。
作業台はconstructor確認済みの同じopeningの入力1..9も受け付ける。result slot 0は
`take_crafting_result` / `transfer_crafting_result`を使う。作業台の付属player slotから入力への転送は対象外。
`Container { screen }`は元の受信済みstorage openingで、storageと付属player slotsを指定する。
1.21.11ではconstructor確認済みのかまど・溶鉱炉・燻製器の全39slotも指定できる。
材料と燃料の受信根拠・転送順・自動精錬との競合は[炉の操作](common-furnaces.md)を参照する。
数字が同じwindowでも新しいopeningの`ScreenId`は元の操作を引き継がない。
従来の`InventoryClickSource`は同じ型の互換aliasで、通常PICKUPにも`InventorySource`を使える。

実mode、元UI、全transfer対象slotの受信済みbaseline、実受信cursor、版別item/slot容量と
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
data付き装備済みarmorも取り出せる。survivalでは束縛の制限を適用し、creativeでは元ゲームの
mode判定に従って取り出す。legacyのlevel getterとmodernの受信enchantment効果を区別する。
カーソルに通常itemを持った状態でもQUICK_MOVEを送れ、その数量・dataは保持する。
結果slotの専用APIと、入力slotの通常転送は別のowner経路を使う。

player入力からは9..44、作業台入力からは10..45へ正順で転送する。装備先への自動配置や
空offhandへの配置は行わず、mergeを空slotより先に試す。保持中cursorは数量・dataとも保存する。
製作入力の取り出しに鎧slotの束縛制限は適用しない。作業台入力5..8も通常の入力slotとして扱う。
製作入力全体の受信済みbaselineを要求し、他の入力の変更は競合に残す。
結果slotの再計算は完了証拠に含めず、sourceと変化した全destinationにfresh受信を要求する。
結果slotだけが更新されてもPendingのままで、閉じた元openingや再利用されたwindowからは送らない。

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
未解決dataを競合として保持する。変化しないcursorは実観測とnative item同値を検査するが、新しいcursor packetは
nativeが送らないことがある。`cursor_inspected`はその元の実受信ordinalを保持し、新規受信と扱わない。

legacyのnative returnは最初のroundで完全に移るならEmpty、partial/internal round後は
元roundのpredecessorになる。独立native oracleでこの値も照合する。異なる実比較値を1回のpacketへ
載せてfull resyncを求め、元window/actionのfreshな実replyと全destinationを待つ。
false comparison replyはgameplay rollbackではない。modernは予測changed mapを送らず、
受信baselineとdefault cursorのnative比較値を使って実changed slotsを要求する。
data付きcursorでは、保存した実revisionと異なる送信revisionを明示してfull contents/cursorを要求する。
この場合のEmpty比較markerを実カーソル値・カーソルhash・受信証拠として扱わない。

`inventory_transfer_record()`の`ObservedTransferred`を確認してから次へ進む。
send executorの応答・全量境界がI/O前の受信在庫と等しく`Diverged`/`NoEffect`で終わった場合は、
changed slotのreceiptが一つもない記録を終端`ObservedUnchanged`にする（通常clickと同じ規則）。
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
このrunの検証範囲は変更したequippableの装備先への転送。装備済みarmorの取り出しは下記で別に検証する。


装備済みarmorのdata付きQUICK_MOVEも両版の共通Clientへ接続した。
`survival().transfer_inventory(InventorySource::Player, 5)`などは束縛の制限を適用し、
`creative()`は元ゲームのcreative判定に従って取り出せる。拒否は送信前に`InvalidInput`で返し、
intentを作成しない。legacyは最初に一致した束縛IDと元の数値getterでlevelを判定する。
modernは受信したenchantment registryの`minecraft:prevent_armor_change`効果を使い、
名称やlevelだけで決めない。stored_enchantmentsはこの判定に使わない。

`export_armor_transfers.py`は同じ引数で再生成でき、旧版276件・新版156件の実mayPickupと
全46slot結果を保存する。両mode、armor4slotとmain/offhand対照、旧版の数値型・非有限値・
省略namespace・重複の順序、新版の実enchantment/stored_enchantmentとlevelを含む。
この純粋なmenu試験のmetadata照合はcompoundのwire順を無視し、数値型と浮動小数点bitsを保持する。
独立したネットワーク受信間のNaN等値判定を変更しない。共通consumerの試験は両版・両modeで、
送信前拒否、sourceとdestinationのfresh実受信、旧版の実比較応答待ちを確認する。

実接続は`trial-1.16.1-490c41f0` / `trial-1.21.11-2c9973b6`で、
両mode計4回の装備取り出しとサバイバルでの束縛拒否2件が成功した。元serverのRCONで数量・
Damage=7・marker=992を独立して照合し、拒否時のクリック送信がないこともtraceで確認した。
旧版の前ケースのstackはhotbarに実転送して対照として保持し、`/clear`から空receiptを推測しない。
両JVMはexit0で、実行時の全runtime source/dataと同一consumer binaryのhashを保持する。


保持中cursorの転送は`export_held_cursor_transfers.py`で同じ引数を使って再生成する。
両版720件ずつ、playerと全9storage menu、両方向・両mode、空/部分merge/blocked destination、
空/stone/dirt/default helmet/data付きenchantment helmetのカーソルを照合する。
元menuの全slot結果はEmptyカーソルの対照と一致し、各ケースのカーソルは保持された。
cursorだけにdataがある場合も保持したregistryで意味を解決し、送信前と受信後の数量・data変化、
configuration変更はinspectionに残す。NBTのcompound順やdefault componentの明示を同値として扱う。

実接続は`trial-1.16.1-f61ddc3f` / `trial-1.21.11-cffc14b9`で両modeとも成功した。
共通PICKUPでDamage=7・enchantment・marker=992付きhelmetを保持し、別のdirt7個をQUICK_MOVEし、
共通PICKUPでhelmetをslot10へ返却した。転送4回と取り出し/返却8クリックで、実slot/cursor受信と
旧版の実比較応答を確認した。RCONはmenu cursorを直接取得できないため、在庫数量と返却後の
Damage/markerを独立して照合した。両JVMはexit0で、同じconsumer binaryとruntime source/data hashを保持する。
実接続後に変更したのは`operations.rs`のAPI説明2行と`capabilities.rs`の条件文4件だけで、
feature/support区分や通信・操作・受信処理は保持した。元の実行snapshotとこの説明変更を区別して記録する。


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


製作入力の原版fixtureは`export_crafting_input_transfers.py`で上と同じ引数を使って再生成する。
両版各2,496件で、player全4入力・作業台全9入力、両mode、容量64/16/1、装備可能item、
空きslotより後方のmerge先、満杯・部分転送・複数destination、同値/異値dataと保持中cursorを照合する。
原版コピー時のNBT compound wire順の差は、数値型を保持したdecoded treeで比較する。
旧版はnative client-side guard、新版は空のnative recipe managerと未接続のnative connectionを使う。
ゲームmethodは置換せず、実recipeの再計算・network受信・取消の根拠とは分けて扱う。
packet試験は結果slot単独更新、全destination/sourceの受信、競合の復元、旧openingを検査する。

```bash
python3 -B scripts/export_crafting_input_transfers.py \
  --downloads "$DOWNLOADS" --modern-classpath-file "$MODERN_CLASSPATH" \
  --runtime-output .local/crafting-input-transfer-oracle --check
```


公式1.21.11のliveは両mode・両gridの全量/部分転送、計8ケースで成功した。
custom data付きplanks7個を回収し、満杯時は63/62個のstackへ3個だけmergeして入力4個を残す。
全changed slotのfresh受信と元QUICK_MOVE1packetを照合し、結果slotのbutton再計算は別に保持した。
保持中helmetのDamage=7とcustom dataは明示返却後にRCONでも照合した。
[実接続の根拠](evidence/common-crafting-input-transfer-20261022.json)に元packet、RCON、receipt、
公式JAR・consumer binary・source/artifact hashを保存する。旧版liveや取消/packet lossをこのrunの証明には含めない。
一時serverは終了済みで、供給元server directoryは変更しない。

```bash
cargo build --locked --example climbing_control_probe -j2
python3 -B scripts/run_crafting_input_transfer_live.py --accept-eula \
  --server-jar "$SERVER_JAR" --output .local/crafting-input-transfer-live
```
