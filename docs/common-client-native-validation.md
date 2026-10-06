# 共通Clientのnative検証

`examples/common_native_probe.rs`を、公式vanilla 1.16.1と1.21.11へ同じコードで接続する。
利用側の版選択は`ConnectionConfig::offline_from_env`だけで行う。
版ごとのサーバー起動設定・gamerule名はPython側のfixtureに閉じ込める。

```bash
python3 scripts/run_common_native.py --all --accept-eula
```

Python 3.8以上、Java 21、Cargoとネットワーク接続が必要。
`--accept-eula`は起動する公式サーバーのEULAへ同意している場合に指定する。
`--runtime-dir /dev/shm`を指定すると、使い捨てruntime/worldをメモリ上で動かせる。
この環境の最終試験はこの設定を使用した。runtimeは両版とも100MiB未満で、JVMは同時起動しない。
JVMを回収してから`.local/native-client-unification/`へ記録とworldをコピーし、一時領域を削除する。
ディスク上では切断確認のRCONがtimeoutし、終了時のdumpに保存I/O待ちが残った試行もある。
その試行は強制終了した失敗としてhashと理由を保存し、成功したrunへ上書きしない。
この設定の成功はディスク永続化や終了保存の耐障害性の検証ではない。

単独版の実行は`--version 1.16.1`または`--version 1.21.11`を使う。
`--all`は1.16.1の終了後に1.21.11を起動する。ビルドもサーバー起動前に`-j1`で完了させる。
同時に別のビルド・検証サーバーを起動しない。

## A1の一連操作

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario basic-workflow --accept-eula --runtime-dir /dev/shm/voxrig-a1
```

各版でSurvival、Creativeを順番に実行する。それぞれ一つのClientで接続・観測・有限地上歩行・
single chestへのstone 2個の収納・player 2×2でstick 4個の製作・dirtの設置・切断を通す。
modeの受信後にhotbar 0を明示的に選ぶ。fixture準備の後はcontrollerから状態を上書きしない。
移動の実終点、チェストの内容、材料と製作結果の在庫、設置blockをRCONで独立確認する。
Client側でも同じsession、実cursor、入力/result grid、変更slot、設置結果を確認する。
Survivalの設置ではdirt 3→2、Creativeでは3個を維持する。

画面closeは完全送信と実受信履歴を区別し、echoのないclose後でも通常製作と設置へ進む。
移動は予測契約で、終点を実受信poseへ書き換えない。両modeの同じ地上歩行入口を使い、
飛行後の立位操作、採掘後の復旧、レシピブック配置や全機能の統合完了はこの試験で主張しない。
`--scenario full`（既定値）は従来の操作別corpusを実行する。

2026-10-06のA1試験は`trial-1.16.1-301ed803`と`trial-1.21.11-32a8a663`で成功した。
同じconsumer binaryとsource/dataを使い、両modeの結果とJVMのexit code 0を確認した。
保存した各`report.json`に入力hash、操作記録、実サーバー側の結果を保持する。

## A2の基本装備とentity操作

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario equipment-entity --accept-eula
```

両版・両modeで同じconsumer、一つの接続を使う。bootsのfeet転送はcommon slot 8の実受信と
RCONの装備で確認する。empty handの一回の攻撃はnative sheep HPの8→7、一回のvillager操作は
実merchant OPENで確認した。送信結果とゲーム内の変化を区別し、merchant layout/取引は成功扱いにしない。
試験側でsheepを削除し、despawn受信後に古い対象への要求がentity frameを送らないことも確認する。
既知typeと固定座標に合う最新spawnの選択はconsumerのfixture規則である。

2026-10-06の成功runは`trial-1.16.1-2617c684`と`trial-1.21.11-c41dad1a`。
同じsource/data/binaryで実行し、両JVMのexit code 0とproxyのエラーなしを確認した。
今回はdisk上のruntimeで成功した。source/dataのhash・実操作記録・RCON結果は各`report.json`へ保存する。
先行した`trial-1.16.1-e126a0ce`は、Creativeのconsumerが前の試験のdying villagerを選び、
削除された元のtargetとして拒否されて失敗した。失敗記録を保持し、最新のmatching spawnを選ぶ修正後に再検証した。
共通APIの範囲と制約は[基本装備とentity操作](common-client-entities.md)を参照。

## A3の採掘復旧と次の設置

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario mining-recovery --accept-eula
```

同じconsumerで、通常完了と早いFINISHの未解決状態の二経路を両版へ通す。
どちらも元接続を閉じ、一度だけ同じprofileで再接続し、新しい受信・standingを検査してから設置する。
初期fixture後のRCONは読み取りだけで、資材3→2、位置不変、設置blockを照合する。
未解決経路ではFINISHの処理応答を受信してからcloseし、元の採掘の予定時間を過ぎても
元のstoneが残ることを確認する。wire上の元UUID/name、新しいconnection、各経路の
START/FINISH各1回とfresh設置1回を確認し、旧ID・元接続・watch cloneからの再送を拒否する。

2026-10-06の最終成功runは`trial-1.16.1-0d4120cb`と`trial-1.21.11-3d05b30f`。
各版の二経路、同じsource/data/binary、両JVMのexit code 0、proxyのエラーなしを確認した。
先行した通常完了だけの試験`trial-1.16.1-3e53c90b`と`trial-1.21.11-11202850`も成功記録として保持する。
この区切りの全体テストは単体686件（8件ignored）、公開API2件、doctest24件が成功した。
詳しい契約・制約は[採掘後の復旧](common-mining-recovery.md)を参照。

## 検証する結果

| 共通APIの操作 | Voxrigとは別のサーバー側確認 |
| --- | --- |
| creative hotbar write・選択 | RCONのInventoryがslot 1のstone 3個、SelectedItemSlotが1 |
| 許可済みflight step | RCONのPosが`[0.5, 66.0, 0.5]` |
| creative break | 対象`[0,65,1]`がair |
| creative use-on-block | 隣接対象`[1,65,0]`がstone |
| survivalへ変更した後のcreative write拒否 | Clientが拒否し、RCONのInventoryにもdiamondが出現しない |
| survivalの35tick read-only preview | fresh teleport後に同じ型の予測を取得し、前後のRCON Posが`[0.5,65.0,0.5]`のまま |
| survivalのread-only first outline | 選択cellが実際のstoneで、query前後のRCON Posが同じ。面/交点は別のnative-method oracleと照合 |
| survivalのstone START/FINISH | 新しい接続でfresh target airを受信し、RCONでもair・位置不変を確認 |
| survivalのdefault dirt設置 | さらに新しい接続でfresh target/materialを受信し、RCONでもdirt・材料3→2・位置不変を確認 |
| survivalのoccupied player swap | main stone 3 / hotbar dirt 2を交換し、両slotのfresh受信とRCONのslot/item/countを照合 |
| creativeのempty hotbar swap | 同じ接続で受信mode変更後にmain dirt 2を空hotbarへ交換し、両slotのfresh受信とRCONのslot/item/count、位置不変を照合 |
| container画面の実内容 | 新しい接続でcreative interactionからsingle chestを開き、stone 3とplayer dirt 2のslot対応、cursor/full contentsを共通APIで観測 |
| 両modeのstorage狙い判定 | 同じtarget_blockでsingle chestのnative inset/North面を読む。RCON Pos/Rotation不変を照合。面/交点は別の公式JAR oracleによるmodel検証で、server hit ACKではない |
| 開いたcontainerへの外部変更 | RCONでchest slot 0をstone 7へ変更し、同じopeningに新しいslot受信が届くこと、native items/位置不変を独立照合 |
| survivalのstorage取出し | 同じopeningのstone 7を空hotbarへSWAPし、両fresh receiptと独立RCONの空container/stone hotbarを照合 |
| creativeのstorageへ戻す操作 | 同じopeningでmodeの実受信後にstone 7を戻し、両fresh receiptと独立RCONのcontainer/player contents・位置不変を照合 |
| creativeの共通storage open | empty-handの同じopen_containerでsingle chestを開き、実OPEN/full/cursorとmodern processingを保持。RCONはstone 3/位置を照合し、targetへの因果関係は主張しない |
| creativeのcontainer closeとsurvival再OPEN | 元のScreenIdへのcloseを一度だけ送信。外部でstone 11へ変更し、実survival mode受信後に同じopen_containerで別の実opening/full/cursorを受信。close送信からACKや受信screen消去を捏造しない |
| survivalのcontainer close | 新openingからcloseを一度だけ送信し、再クリック/再closeを拒否。独立RCONはcontents/inventory/位置を照合し、menu stateを確認できたとは扱わない。切断後もclose recordを保持 |
| close後のsurvival player交換 | complete no-echo closeのlocal player UI根拠を保持し、同じswap_hotbarでmain dirt 2を空hotbarへ交換。両fresh receiptと独立RCONのplayer slotsを照合 |
| close後のcreative player交換 | 受信mode変更後に同じAPIでdirt 2をmainへ戻す。actual window/revision/cursorの捏造なし、chest stone 11/位置不変と独立player Inventoryを照合 |
| survivalの有限jump/歩行 | RCONで途中の高さ・水平移動を取得し、実終点が予測終点に一致 |

共通Clientは実際の受信mode・teleport・対象blockを待ってから操作する。
pitch範囲外、4blockを超えるflight、creative modeでのSurvival handleのmutationも拒否を確認する。
`DispatchReceipt`は送信結果に限る。除去・設置ではClientの受信blockも待つが、
最終判定には別経路のサーバーRCONを用いる。

## 実行環境と記録

Mojang version manifestのmetadata SHA-1とserver JAR SHA-1を照合してから実行する。
runtimeや使い捨てworldは`.local/native-client-unification/`へ保存し、packageへ含めない。
各runの`report.json`、`probe.jsonl`、`probe-stderr.log`、`server.log`を残す。
RCON passwordは公開記録へ含めない。

サーバーはloopback・offline mode、heap上限1024MiB、ActiveProcessorCount=1、view distance=2。
fixtureのchunkをforceloadし、配置をRCONで確認する。初期spawn位置に依存せず、
Client接続後にfixtureへteleportして新しい実受信poseを待つ。
1.21.11の[gamerule名変更](https://www.minecraft.net/en-us/article/minecraft-java-edition-1-21-11)もbootstrapで扱う。
構文エラーや未ロード位置へのfixture commandは失敗として扱う。

使い捨てworldでは`sync-chunk-writes=false`。
この環境ではtrue時に終了時のRegionFile header書き込み待ちが続き、正常終了できなかった。
JVMのSIGQUIT thread dumpでIO workerの`pwrite`とserver threadの保存完了待ちを確認し、
falseでは両版が正常終了した。これはワールド永続化の耐障害性を検証する試験ではない。

Clientの明示的disconnectとサーバーのexit code 0を両方要求する。
遅い終了はthread dumpを残して期限付きで停止し、強制終了が必要なら操作が成功しても
run全体はfailedとする。次版は前版のprocessを回収してから起動する。

コミットされた[結果の抜粋](../data/client_api/common_native_evidence.json)にはJARの出所、
検証コードのhash、独立確認の結果と終了codeを記録する。
これは上記基本操作・有限dry移動・限定read-only狙い判定・採掘/default cube設置・default player stack交換の検証であり、さらに広い移動・採掘・設置条件、一般container、crafting、
複雑なitem data、entity、復旧などの残作業を完了扱いにするものではない。
previewの取得は実際のsurvival移動を検証するものではない。

有限survival移動も同じconsumerで35tickのjump/歩行を送信する。実行中にselect/二重startが拒否されること、
全35tickが送信されたこととown-pose receiptを終点で置換しないことを検査する。
controllerは途中のPosをRCONで複数回取得し、1block以上の上昇、水平移動、予測終点と実際の終点の
各軸1e-7以内の一致を確認する。サーバー受理のscenario検証であり、すべての物理条件やfresh observer契約の保証ではない。
各runのraw reportにはnative position samplesを保存する。

read-only狙い判定も同じconsumerで実行し、creativeで設置したstoneの最初のoutlineを取得する。
RCONでは対象stoneとPos不変を確認する。面/交点が正しいことは別の[公式JARのnative-method oracle](common-survival-targeting.md)で検査し、
このRCON確認をserver自身のtarget receiptと扱わない。続いて別の新しい接続でsurvival採掘を行う。保持した有限移動runから暗黙に復旧しない。
creative writeが送信者へ返送されない場合もあるため、移動試験前の`clear`は実際の在庫更新を受信させるfixture操作である。
Clientの未解決在庫markerをRCON確認で消したり、previewのguardを回避したりしない。

採掘は新しい接続の受信mode・own pose・空のcursor/selected slot・target stoneを待ち、
明示的なSTARTとFINISHを行う。estimated waitはローカルの待機目安だけとし、
結果はClientのexact target air受信と独立したRCONの対象airで確認する。
重複FINISH、競合look、air確認後の操作継続も拒否される。元の受信poseと閉じた接続の履歴を保持する。
共通fresh recoveryは後続段階に残る。

設置は採掘のsource切断後、さらに新しい接続から同じ`place_cube` / `placement_record`で実行する。
受信済みのdirt 3個、空cursor、own pose、stone支持blockと隣接airを確認してから一度だけ送る。
同じ場所への二重設置を拒否し、対象dirtと材料2個の実受信を待つ。modernでは実processing ACKも確認する。
RCONで対象dirt・在庫2個・位置不変を別に照合し、切断後も完了した診断を読み出す。
packet fixtureでは完了後の次の場所での設置、取消、transientな足場/材料競合、古いACKも検査する。
legacyには存在しないprocessing sequenceを作らない。任意形状・複雑な材料dataの対応はこの検証に含めない。

メモリ上の速いfixtureでも、teleportのown-pose受信とlocal grounded geometryの成立は別である。
採掘前のread-only target queryで条件が整うまで待ち、stand guardを回避しない。

在庫交換はさらに別の新しい接続で行う。版別のreplaceitem/item commandでfixtureを準備し、
Clientの実受信とRCONの初期slot/item/countを両方確認する。survivalでoccupied pairを交換し、
同じ接続で受信modeをcreativeへ変更してempty destinationとの交換を行う。
結果は両slotのfresh received値と独立したRCON Inventory、位置不変で照合する。
legacyはnonempty received predecessorをcomparisonとして送り、nativeのnegative比較応答・full resyncを受ける。
false比較応答は交換を取り消した証明ではない。modernにはこのtransaction ACKを作らない。
初回のlegacy fixture slot名誤りと、正しいEmpty returnによるslot更新抑止はfailed runとしてhash/理由を保持する。
ライブラリのguardをRCON結果で解除したり、slot予測を受信値へ変換したりしない。
詳細と公式codecの検証は[共通在庫交換](common-inventory-swaps.md)を参照する。

共通storage activationのnative検証は同じ接続でcreative open、storage交換、close、
survival再openを実行する。`container_open_completed`と`container_reopen_completed`に
完全送信・actual新OPEN・fresh full contents/cursor・modern actual処理sequenceを記録し、
切断後のopen historyも確認する。legacyに処理ACKは作らない。OPENにblock座標がないため、
matching screenの実受信とtargetがそれを開いたという因果関係を区別する。
[共通open契約](common-container-open.md)のempty-hand・dry standing・shape/menu範囲の検証であり、
ロック、遮蔽、animated shape、general UI/item useを完了扱いにしない。

barrelも同じClientの両modeでopen/closeする。native `open=true/false`とstone 5・player在庫・
位置不変をRCONから確認し、Client側は実画面・内容・cursor・modern processingと実受信world
cacheのopen flagを別に観測する。通常のopen flag変更を未知のgeometry変化へ読み替えず、
facing等の別property変更はconflictとして保持する。最初のchest-only native成功はこの追加
barrelシナリオを検証したものではなく、後続の全体runを最終evidenceとして保持する。

native slot条件・item容量・default cursor hash encoderを揃えた後の回帰runは、
`trial-1.16.1-63556176` / `trial-1.21.11-e3596fc0`。両版とも同じClient consumerの
storage/player交換・開閉を含む全シナリオが成功し、JVMはexit 0、tmpfs runtimeは削除済み。
`data/client_api/regular_click_native_evidence.json`に実RCON結果、fresh swap receipts、
実行時input hashとraw report hashを保持する。以前のfailed runと履歴は元のevidenceに残す。
この2 runは通常PICKUPの接続/owner/受信契約を検証したrunではない。
後続の`trial-1.16.1-3275293e` / `trial-1.21.11-a30b17c0`では共通PICKUPも実行し、両JVMはexit 0、
tmpfs runtimeは削除済み。chest stone 7をsurvival右クリックでsource 3/cursor 4へ分割し、
creative右クリックで1個戻して4/3、左クリックで全量を戻して7/Emptyを実際に受信する。
close後のplayer main/hotbarも、survival取り出し、creativeで1個置く・返却・再取り出し・元の在庫への
復元を実行する。RCONは独立したchest/player数量と位置不変を確認する。cursorは実packetからのみ
確認し、RCONのcursor/menu所有確認とは扱わない。新しい実行時input、8完了recordずつ、raw hashと
RCON結果は`data/client_api/ordinary_pickup_native_evidence.json`に保持する。
元menuのPICKUP primitiveとslot条件の別の照合は[通常クリック調査](common-inventory-clicks.md)を参照。


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

追加のcursor処理調査は`trial-1.16.1-5171bb66` / `trial-1.21.11-fa5a1f79`で成功した。
同じconsumer binaryで従来の全workflowと各版9件の完了Shift操作を再確認し、
さらに両modeの新しい実接続でbarrelからstone 5をPICKUPして、範囲外への外部teleport後に
vanillaが元の画面を閉じるまで観測する。通常の圧縮通信をそのまま転送するreadonly traceには
元windowの実CLOSEがあり、各調査接続にClient closeの送信はない。
独立RCONでlegacyのstone 5 item entityと、modernの在庫返却／item entityなしを確認した。
両JVM exit 0、tmpfs runtime削除済み。元source/dataをbuild前にhashし、実consumer binaryと
raw report/trace/hashは`data/client_api/cursor_close_native_evidence.json`へ保存した。

初回の強制close後のcursor/UI不足と、同値再受信によるlegacy送信前の誤拒否もfailed runとして残す。
前者は新しい実接続を使うfixtureへ修正し、後者は受信値とhistory ordinalを分けるlibrary修正を行った。
実値の競合が復元されても解除しないこと、再受信を送信後の結果として数えないことをtransport試験で確認する。
この調査はcursor付き共通closeの実装・完了ではない。終了前の実在庫返却／実受信／owned closeを
次に組み合わせる。範囲と順序は[追加native調査](common-cursor-close-audit.md)を参照。

## 実cursor返却からcloseまで

`trial-1.16.1-8aa5526a` / `trial-1.21.11-e1c711c9`は同じconsumer binaryで既存の全workflowとcursor付きcloseを通過した。
各版・各modeでstone 5の部分結合→空きmain返却とdefault diamond helmet 1を検証し、modernはdefault bundle 1も検証した。
close record内の各PICKUPは実source/cursor before、予測、完全送信、新しい実source/cursorとlegacy実比較応答を別に保持する。
すべてのstepの実結果とEmpty cursorを確認後にCLOSEを一度だけ送り、vanillaの無応答をACKへ変換しない。
RCONは個数保持、dropなし、空で閉じたbarrel、位置・向き不変を独立に照合する。
両JVM exit 0でtmpfs runtimeを削除した。raw hashとactual input/binary snapshot、結果は
[cursor return native evidence](../data/client_api/cursor_return_native_evidence.json)に保持した。

## 名前付きitem dataの受信

続いて両版のsurvival/creativeで、名前付きstoneとcustom markerを元サーバーに外部設定し、
同じ共通Clientの実受信slotを確認した。1.16.1のNBTと1.21.11のcomponent patchは
受信dataとして保持し、サーバーの独立RCON値と一致する。操作結果ではなく受信の検証である。
初回の切断時traceエラーを保存した上で、共通protocol送信のheader/bodyを一つのbufferに
まとめ、両版の全scenarioを再実行してexit 0・trace errorなしを確認した。
明示した切断後に転送できない完全なclientbound frameは未配達として記録し、
途中frameと送信側の失敗は失敗のまま扱う。最終試験ではこの未配達もなかった。
範囲・再現手順と残作業は[共通item data](common-item-data.md)、実行時の入力hashと
以前の失敗は[item data native evidence](../data/client_api/item_data_native_evidence.json)を参照。

追加のmodern fixtureは名前付き入れ子item、実エンチャント、本の本文を同時に含む。
同じ公開Clientで両modeのfresh slot/patchを受信し、独立RCONが入れ子の個数/marker、
enchantment level、本文と位置/回転不変を確認した。通常の在庫変更をClientから送っていない。
legacyも同じconsumerで再実行し、両版の既存全workflowも成功した。
最初のmodern試験は自作marker照合のlength誤りで失敗し、その元report/hashを保持した。
修正後は両JVM exit 0、trace error/未配達0で完了した。
実行時入力と結果は[complex item data evidence](../data/client_api/item_data_complex_native_evidence.json)。
これは元データの受信検証で、実接続のregistry参照解決・意味の正規化やdata付きitem操作を
実装済みとする証拠ではない。

追加試験は同値pose再受信の誤拒否と、modernの新しいposition公開がteleport確認より先だった競合を発見した。
前者は同じ実位置・向きを比較して履歴ordinalを保持し、後者はoriginal確認/position応答の完全送信までstate lockを保持する。
writerを止めたtransport回帰試験でnormal lookが確認packetを追い越さないことを確認する。
fixtureも外部teleportの新しい実poseを待つ。過去の失敗を成功へ読み替えず、最終runと分けて保持した。
任意item data/components、製作・一般装備・entity、広いmovement条件、context/記録/再構成/復旧等は全体goalに残る。

## Received recipe catalogue

The same public `Client::received_recipes()` observes all native recipe grants,
the 1x2 stick and 3x3 cake arrangements, actual revoke/regrant and identity behavior,
separately in survival and creative. RCON changes the native book; the common probe
uses received entries instead of locally constructed recipes. Legacy retains the
declaration when locked; modern removes it and regrant supplies a new add ordinal.
See [common recipes](common-recipes.md). Planning/placement and inventory eligibility
are separate remaining work. The servers still run sequentially.

## Recipe-book材料の共通観測

`recipe_book_materials`はcatalogue・実tag・main/hotbarを同じadapter capture境界で取得する。
両modeで棒の材料にoak planksを3個与え、1batchの割当と最大1batch、2batchの不足を確認する。
同じstackにcustom nameを与えるとnative simple-stock filterによって寄与0になり、名前を除くと3へ戻る。
fixture commandと各stackの実受信ordinalを別に保持し、実送信やgridの配置成功とは扱わない。
この試験は元stock getter/accountingの72ケース、ingredient pickerの80ケースの照合と組み合わせる。
サーバーは順次実行し、両版の正常終了・同一consumer binaryとsrc/data/Cargo/probe/controller hashを要求する。
現grid material・返却space・UI容量を含む配置plan、recipe-book配置、result merge/shift-craftingは残る。

## Coherent crafting context, layout and grid return capacity

The common native consumer now calls `Client::received_crafting_context()` in
survival and creative on both original versions. For the received stick display,
player 2x2 inputs map to (0,0)/(0,1); table 3x3 inputs map to (1,0)/(1,1). The
context's inventory, grid and catalogue share the adapter packet boundary while
actual slot ordinals remain separate.

The table scenario receives one plank in inventory, one in input (2,2) and one
on the cursor. Its grid-only return plan predicts inventory count two and one
input-to-slot-9 transfer, explicitly `Predicted`; it excludes the cursor. The
existing close operation separately returns that cursor using actual receipts,
then the native menu returns its grid input. Independent RCON verifies three
planks after close, no dropped item, and unchanged player position. Predictions
are retained alongside actual receipts; they never replace them.

Sequential runs `trial-1.16.1-15d95d0c` and `trial-1.21.11-88c4a9f0` passed all
existing native scenarios plus these context/layout/return assertions. Both
original JVMs exited normally with code zero. Their reports retain the same
consumer binary and actual source/data/Cargo/probe/controller hashes. The 102
original geometry/packet cases and 50 destination/resource-transfer cases
provide separate primitive evidence. Full grid-inclusive placement planning,
recipe-book dispatch, result merge and shift crafting remain follow-up work.

The return preflight now preserves positive over-limit counts instead of treating
them as unavailable. Original legacy clearing offers individual units; modern
returns split by item capacity and keep one destination while inserting that copy.
The primitive corpus has 64 cases and 126 player/table comparisons, including
damaged tools and modified capacities. It separately runs the entire original
modern return handler for 22 successful owner-free cases. In native stream-only
capacity-128 cases, fixed-destination copies can remain uninserted despite other
free slots. The common diagnostic retains those copies in `unreturned_splits()`
and reports `fits() == false`; it never calls an unused owner/drop path or claims
an actual loss receipt.

The live table consumer additionally calls the mode handle's `select_hotbar(2)`
before capturing the return plan. Its basis remains `Submitted`, and RCON
independently verifies actual native `SelectedItemSlot` 2. The existing actual
cursor return and grid close still produce three planks without drops in both
modes/versions. Sequential runs `trial-1.16.1-e5a3287d` and
`trial-1.21.11-bd1851f1` passed all scenarios; both original JVMs exited zero.
Original registration also establishes recipe-placement packet IDs 0x19 and
0x26 respectively. Codec/registration evidence does not implement or authorize
recipe placement; full placement planning and owned dispatch remain pending.

Sequential runs `trial-1.16.1-93038b7a` and `trial-1.21.11-6d198fe4` verify coherent
`recipe_placement_plan` from the same public Client/binary in both modes. Next
and Maximum see three actual planks as one material batch; custom-name presence
excludes the stack and name removal restores the preflight. The controller
independently checks the captured plan/session/mode/recipe/receive boundaries
and that these read-only calls emit no recipe-placement request. Existing live
scenarios, actual result takes, cursor/grid returns, RCON inventory and no-drop
evidence also pass; both original JVMs exit zero. This is placement preflight
evidence, not owned placement dispatch, placement acknowledgement or consumption.
The entire original shaped/shapeless matchers separately contribute 102 cases.

## A4の記録・読み取り専用再生・限定scene

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario recording-scene --accept-eula --runtime-dir /dev/shm/voxrig-a4
```

両版で同じconsumerが`Client::connect_recorded`から受信を記録し、完全なJSONを保存して
`PacketTrace::replay`で読み取り専用再生する。元の全受信packetのphase・ID・payload hash・ordinalを
透過proxyで独立照合し、player pose/dimension/mode/health、player inventoryの全slot/cursor・
NBT/componentデータと元の受信ordinal、選んだblockをlive観測と比較する。
RCONで実position、独自metadataを持つplanks 3個とblockを独立確認する。

共通のscene captureから34tickの有限予測を行い、同じ版のlive previewと全frame・seed・
terminal clearanceが一致することを確認する。初期halo不足・空入力・region外の予測を拒否する。
その後、実serverの別cellを変更し、元sceneが変わらないことを確認する。
source切断後も同じ記録の再生とdetached予測が同じ結果を返し、live captureは拒否する。
読み取り専用操作からgame mutationが送られていないことも送信packet IDで確認する。
認証packet、全protocol履歴・entity現在状態・広いpiston再構成・scene編集/連鎖の共通化は
この限定したA4の証拠に含めない。

2026-10-06のA4試験は`trial-1.16.1-ea67f007`と`trial-1.21.11-88294321`で成功した。
同じ400個のsource/data/consumer/driver入力と同じbinaryを使い、両JVM exit0、proxyエラーなし。
接続開始からの記録91packet／95packetの元payload hash・phase・ID・ordinalをすべて照合した。
同じ34frameのnative予測と限定sceneの全frameが一致し、実地形更新・切断後の同一予測と
同一受信再生を確認した。raw recording・独立RCON結果・全入力hashは各runに保持する。
単体690件（8件ignored）、公開API2件、doctest26件、proxy境界3件とall-target Clippyが成功した。

## A5のscoreboard観測とClientManager

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario manager-ui --accept-eula --runtime-dir /dev/shm/voxrig-a5-ui
```

同じconsumerからmanagerが2つの名前付きClientを生成し、それぞれのplay readinessを待つ。
objective・sidebar・2つのscoreを両Clientで観測し、各値の受信ordinalから元packet全fieldの
長さ・SHA-256を照合する。scoreの7→9更新、owner全objective reset、objective削除を実受信で確認し、
値9と接続人数2→0を独立RCONでも検査する。重複name/profileは追加loginを発生させない。
managerの明示的shutdown後は外部に保持した両Clientも閉じ、新規接続を拒否する。
proxyの切断許可は実接続ごとに指定し、一つのClientの正常終了で他の接続の異常を隠さない。

2026-10-06の成功runは`trial-1.16.1-09bd195f`と`trial-1.21.11-b61b968e`。
同じ402入力・consumer binaryで各版の実login 2件と元field照合16件、両JVM exit 0、
proxyエラーなしを確認した。両版は同時起動していない。
先行した`trial-1.16.1-c102041c`は新共通decoderのlegacy score packet ID誤りで失敗した。
既存native decoderと元受信packetに合わせて0x4dへ修正し、失敗記録を保持したまま再検証した。

生成ごとのmixed-version／registry分離、接続取消・shutdown中のpending transport閉鎖は
両版の軽量TCP fixtureで検査する。異なる版のJVMを同時に起動した証拠ではない。
これはA5のscoreboardとmanager部分の検証。かまどslotと実乗車／下車の検証は後の節に記載する。
number formatの3種と任意displayの保存はcodec単体試験で確認し、実サーバーで全表示形式を
操作したとは扱わない。契約は[基本UIとmanager](common-ui-manager.md)を参照。

## A5のかまど基本slot操作

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario furnace --accept-eula --runtime-dir /dev/shm/voxrig-a5-furnace
```

各版・各modeで一つのClientから、同じAPIで通常のかまどを開き、fuelを先に入れ、
iron oreを置き、新しいiron ingotの実受信を待ち、取り出してplayer在庫へ戻し、close・切断する。
各clickのsource/cursorとlegacy実reply、同じsession/opening、6件のPICKUP frameを確認する。
初期fixture後はRCONで編集せず、燃焼中のblock、材料→結果、空になったかまどと在庫の
iron ingot 1個、位置不変を独立確認する。closeは送信結果でありscreen echoを捏造しない。
close後の古いscreenへの操作が追加clickを発生させないことも確認する。

2026-10-06の成功runは`trial-1.16.1-5cda8b49`と`trial-1.21.11-d06e282f`。
同じ407個のsource/data/consumer/driver入力と同じbinary、両mode成功、両JVM exit 0、proxyエラーなし。
溶鉱炉・燻製器を含む39-slot topology、vanilla燃料判定・bucket容量、24 block state・
144 native clipは、別の公式JAR observerへ照合している。特殊レシピ・XP・燃焼時間の予測や
custom datapack semanticsをこのlive試験で検証したとは扱わない。

先行の`trial-1.16.1-d4042a9c`は周辺geometry受信前のopenで停止し、読み取り専用の
狙い判定を待ってから一度だけ開く試験へ修正した。`trial-1.21.11-d16fce76`はSurvival成功後の
Creative fixtureに以前の燃焼時間が残ったため失敗した。元受信の二回目のopeningにも
正のnative燃焼propertyが残っていた。各mode前にblock entityを作り直してから再検証し、
両失敗runの記録も保持する。契約は[共通かまど操作](common-furnaces.md)を参照。

この変更の回帰検査では単体699件（8件ignored）、公開API2件、doctest26件、
all-target Clippy `-D warnings`、Rust 1.85.0のlib checkが成功した。

## A5の乗車関係の受信経路

`Client::vehicle_state()`へ両adapterの実SET_PASSENGERS decoderを接続した。
共通ledgerとadapter fixtureで、未知・実乗車・同じ車両からの実除外、別車両の無関係なlist、
spawn寿命／ID再利用／world reset、不正packetの原子的拒否、下車後にも続くground admission拒否を確認する。
modernの不正packetは接続のProtocol failureを保持し、以後の受信とlive captureを拒否する。

これは受信経路と共通観測の検査で、実サーバーの乗車→owned下車workflowとは区別する。
両版の公式JARは元passenger packetとinput serializer／handlerを読み取り専用で調査した。
[JARと元inspectionのhash](evidence/common-vehicle-observation-20261006.json)を保持するが、
そのinspectionだけで下車操作をnative検証済みとは扱わない。
owned下車送信・結果record・両版の実乗車→下車は次節で検証する。
観測契約は[共通乗車関係](common-vehicles.md)を参照。

乗車観測追加後の回帰検査は単体704件（8件ignored）、公開API2件、doctest27件、
all-target Clippy `-D warnings`、Rust 1.85.0 lib checkが成功した。


## A5の一回の下車要求と実受信

`--scenario vehicle`は同じpublic consumerから各modeで一つのClientを保ち、
受信したminecart spawnへ空手で一度INTERACTし、実乗車→owned下車入力→実除外→明示的neutral→切断を行う。
固定fixture後のRCONは読み取り専用。乗車時の元player `RootVehicle.Attach`をcartの実UUIDと照合し、
実除外時にはplayerが接続したままでRootVehicleがないことを独立確認する。
playerはvehicleのsaveAsPassenger NBTに保存されるとは限らないため、車両のPassengers NBTは証拠に使わない。

成功runは`trial-1.16.1-677755d2`／`trial-1.21.11-52997305`。
同じ414 source/data入力とconsumer binaryを使い、各JVMはexit 0、proxy errorなし。
検証後のCargo.toml差分はobserverの2ファイルをpackage includeへ追加するだけで、
依存・build設定・sourceに変更はない。この差分を証拠manifestへ分けて保持し、元の実行入力を更新しない。
各modeでINTERACTは一回、下車入力は要求とneutralの二frameのみ。
元のSET_PASSENGERS fields／受信ordinalとopaque mount寿命を照合し、proxy上の
乗車→要求→実除外→neutralの順序も確認する。重複要求／解除と下車後の地上previewは拒否する。

旧版のPLAYER_INPUTはneutral axesとshift flag、modernはInput shift bit。
[元packet codecの観測](../data/client_api/vehicle_input_source.json)と
[元JAR／読み取り専用inspection・native証拠](evidence/common-owned-dismount-20261006.json)を保持する。
要求とneutralをtick前に続けて送らず、実除外を受信してから解除する。
呼び出し取消後にもactorが一回のintentを保持し、writerが詰まっていても記録は読める。
単体fixtureでは早い解除、mode違い、他車両のlist、他乗員変更、重複送信を検査する。
初回native試行は独立RCON queryのselector構文の誤りで失敗し、修正後に両版・両modeを検証した。

これはA5の代表的な乗車／下車操作の検証であり、要求との因果ACK・操縦・車両physics・地上復旧を意味しない。
その広い範囲はBに残る。観測・送信の契約は[共通乗車と下車](common-vehicles.md)を参照。

下車追加後の回帰検査は単体709件（8件ignored）、公開API2件、doctest28件が成功。
all-target clippy、Rust 1.85.0のall-target check、rustdoc `-D warnings`、fmtも成功した。
配布allowlistには新しいsource/dataとobserver toolを含め、開発用directoryを混ぜない。

## B3の乾いた階段・ハーフブロックと収納

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario dry-terrain --accept-eula --runtime-dir /dev/shm/voxrig-b3
```

同じconsumerの各modeで、底slab→full cube→stairs→高さ67のplatformを44tickで歩き、
一つのClientのまま隣のchestを一度開閉して切断する。Survivalはcaptured sceneとliveの予測も照合する。
fixtureの準備後はRCONを読み取りのみに使い、実終点・収納後の位置不変・空のchest内容を確認する。
元position/input fields、実OPENのwindow/menu/ordinalとmatching closeを別に検査する。
legacyの通常ground loopが前後へ送る待機positionは元traceへ残し、44の有限入力から分けて
初期／終端位置が変わっていないことを検査する。

2026-10-06の成功runは`trial-1.16.1-e98102fd`と`trial-1.21.11-67afb914`。
両modeが成功し、同じ419 source/data入力とconsumer binaryを使った。両JVMはexit 0、proxyのerrorはない。
main `b38e8b4`のPR #10も取り込んだコードで実行した。
[結果と出典](evidence/common-dry-terrain-20261006.json)には、元形状／getterの出典、
各native report hash・受信OPENの証拠と、失敗した3試行の理由も保存する。
先行失敗は立位準備待ちの不足、待機frameを有限操作へ数えた検査、旧版のactivation ID検査の誤りで、
操作guardを緩めずconsumer／verifierを修正した。

これは既知のdry terrainを歩いて収納へ進む区切りである。元clientの全tick物理の同等性、
waterlogged・一般非cube・道具／effect／別姿勢・Creative飛行後の立位操作は完了扱いにしない。

## B3: 地上操作済みClientからCreative飛行へ（2026-10-06）

`creative-flight`は同じconsumerで44tickのdry slab/stair移動→収納の開閉→flight要求→
3つのbounded飛行位置→flight解除→切断まで通す。各版で同じ接続を維持し、
baseline後のRCONは読み取りだけで位置・abilitiesを独立確認する。元packetの
一回のenable、3つのposition、1回のdisableと保持commandを照合した。
元の受信poseを保持し、以前の地上runの終点は診断履歴へ退避する。

1.16.1 `trial-1.16.1-597eb958`／1.21.11 `trial-1.21.11-127651d9`は
同じ422 source/dataとbinaryで成功、両JVM exit 0・proxy errorなし。
明示的な着地→地上操作の継続とvanilla飛行物理全体は、この区切りの検証範囲に含めない。
[共通Creative飛行](common-creative-flight.md)／[入力と結果](evidence/common-creative-flight-20261006.json)。

## B3: Creative着地から地上移動・再収納へ（2026-10-06）

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario creative-landing --accept-eula
```

同じcommon consumerとClientで、44tickのdry slab/stair移動→chest開閉→flight要求と3step→
`move_flying`で既知の床へ戻る→`land`→27tickの新しい地上移動→fresh chest開閉→切断まで通す。
着地は明示的disable／neutralと、宣言したzero controller seedからの2 released ground model ticks。
最初のground falseと次のground trueを元packetへ照合し、受信poseやabilitiesを合成しない。
新しい地上移動は着地終点から予測し、別run IDと送信数、独立したnative終点を確認する。
再収納の実OPENは新しいscreen ID／受信ordinalを持ち、activationとmatching closeは各一回。
fixture後のRCONは読み取りのみで、着地後のflying false・位置・再収納後の空chestを確認する。

成功runは1.16.1 `trial-1.16.1-547b9037`／1.21.11 `trial-1.21.11-001bd50b`。
両版は同じ422 source/data入力とconsumer binaryを使い、両JVM exit 0、proxy errorなし。
初回1.16.1 `trial-1.16.1-cede15da`は再収納closeのproxy到着前に回数を数えた検証側の誤りで失敗した。
実到着を待つよう修正し、両版を再実行した。失敗reportと当時の入力も保持する。
[入力と結果](evidence/common-creative-landing-20261006.json)を参照。

正常なstanding・通常attribute・effect／未解決impulseなし・完全なdry supportに限定した操作である。
自動降下、native飛行物理全体、他姿勢／effect／水中、一般vehicleの地上復旧はBに残る。
受信abilitiesが同じflagsで再到着してもordinalが変われば古い解除根拠を無効にする。
軽量TCP試験は両版でwriter停止中の待機取消、同期診断、一回の解除／neutral／2tick、
後続ground run、新しいabilities受信による拒否を確認する。

## B3: 通常道具の採掘から復旧・設置へ（2026-10-06）

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario mining-tools --accept-eula
```

同じcommon consumerでiron pickaxe／stone、iron shovel／dirt、iron axe／oak planks、
wooden pickaxe／double stone slabの4通りを各版で通す。
各caseは受信道具の選択→START→local scheduling待機→FINISH→fresh target air→
元接続のclose→一回の同一offline profile admission→新しいstone設置→切断まで実行する。
RCONは初期fixture後には読み取りのみで、対象air、Damage 1の道具、stoneの消費と新しい設置、位置不変を照合する。
元START／FINISHとfresh接続の設置frame、profile、旧操作IDの拒否を別に検査する。

最終成功runは1.16.1 `trial-1.16.1-df60e9f2`／1.21.11 `trial-1.21.11-c5beb20b`。
両版は同じ426 source/data入力とconsumer binaryを使い、8 caseが成功した。
両JVMはexit 0、proxy errorなし。単体731件・公開API2件・doctest29件、
fmt／all-target Clippy／Rust 1.85 all-target check／rustdoc／trace／配布検査も成功した。
先行成功runはcapabilityと説明文更新前の入力として証跡で分けて保持する。

元nativeのdefault stack getterからtool speed／correct-tool gateを取得するが、
local model ticksを実server tickや完了ACK、drop保証として扱わない。
耐久値のみのdata・通常属性・健康なdry standing・既知geometryに限定する。
道具に必要なtarget tagが実受信資料と異なる場合は拒否する。
受信target airの後に届く道具の耐久値更新は除去履歴を消さず、先に届いた道具交換やcontext異常は保持する。
元接続のmutationは解放せず、明示的fresh recoveryを要求する。
[入力と結果](evidence/common-mining-tools-20261006.json)／[共通採掘](common-survival-mining.md)を参照。

## B5: 共通Clientの緊急遮断（2026-10-06）

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario connection-revocation --accept-eula
```

同じconsumerで実Clientへ接続し、通常のhotbar選択を一回送信した後に同期で遮断する。
元Clientとcloneの追加選択は拒否する。両handleを保持したままRCONのplayer不在とproxy EOFを確認し、
元の一回以外の選択frameが送信されていないことを照合する。fixture後のRCONは読み取りのみ。
成功runは`trial-1.16.1-5a4b4868`／`trial-1.21.11-cf6a85c5`。
同じ427 source/data入力とconsumer binary、両JVM exit 0、proxy errorなし。
capture／writer停止・別OS thread・partial write・別接続の分離は軽量試験で補う。
今回のEOF観測を、任意の遮断におけるtransport終了やserver静止の保証へ一般化しない。
[契約](common-connection-revocation.md)／[入力と結果](evidence/common-connection-revocation-20261006.json)。

## B4: ownedレシピ配置から結果取得・格納まで（2026-10-06）

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario recipe-placement --accept-eula
```

同じconsumerでSurvival／Creative、player 2x2／table 3x3、Next／Maximumの
8通りを各版で実行する。初期fixtureはplanks 10個と受信book、以降のRCONは読み取りのみ。
sealed planから一回だけrecipe要求を送信し、実入力と在庫の保存を確認した後、
明示的な1回／5回の結果取得とcursor格納を行う。最終grid・result・cursorが空で、
Nextはplanks 8個とsticks 4個、Maximumはsticks 20個のみ。元tableのclose、
同じClientでの新しいhotbar選択、正常disconnectまで確認する。

B4単独の固定入力runは`trial-1.16.1-6239a947`／`trial-1.21.11-c43b7cc9`。
同じ431 source/data入力とbinaryで16ケース成功、両JVM exit 0、proxy errorなし。
全431入力はB4 commit `3f85021`のblobと一致する。後続main取り込み前の固定証拠として保持する。
両adapterの軽量TCP試験ではwriter保持中の履歴取得、待機取消後の一回送信、
実入力だけでは解放しないこと、保存確認後の次操作、old plan拒否、
writer保持中のcommon revocationによる要求の遮断を確認する。
先行成功runと失敗runは証跡に区別して保持する。

製作台は既存のstanding/ray試験と同じ`[0, 65, 2]`に置く。
`[0, 65, 1]`の隣接tableはdry collision modelの未対応としてlookを拒否した。
この制限はB3へ残し、guardを弱めたり成功として扱ったりしない。
ghost／非空cursor結合／shift製作と広いB4は未完了。
[契約](common-recipes.md#owned-recipe-placement-through-the-common-client)／
[入力と結果](evidence/common-recipe-placement-20261006.json)。

main `686f414`を取り込んだmerge `291da13`でも同じ16ケースを通した。
統合後runは`trial-1.16.1-f9cefa8b`／`trial-1.21.11-62556fc0`。
両版は同じ431入力とbinaryで成功し、JVM exit 0、proxy errorなし。
固定B4 commitの証拠と統合後の入力を区別し、後者のsource/data hashも証跡へ保持する。

統合後の単体749件、公開API4件、doctest30件は成功（専用環境等の8件ignored）。
照明／診断／仮想transitionのmain側の追加試験もこの統合suiteへ含める。
fmt／all-target Clippy／Rust 1.85 all-target check／rustdoc／trace境界／配布allowlist・Cargo package buildも成功。
配布対象は772ファイルで、利用側logsと私的な`.local`のruntimeを含めない。
