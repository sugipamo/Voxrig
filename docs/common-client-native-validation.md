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
これは上記基本操作・有限dry移動・限定read-only狙い判定・採掘/default cube設置の検証であり、さらに広い移動・採掘・設置条件、container、crafting、
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
