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
