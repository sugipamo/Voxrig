# カーソルitem付き画面終了のnative調査

この段階はnativeの調査と、legacyの送信前に同値の再受信を競合としていた不具合の修正まで。
当時の共通`close_container`は実受信のEmpty cursorを要求した。現在の返却→受信→closeの実装は
[共通コンテナclose](common-container-close.md)を参照。
item付きcursorの共通close、任意NBT/components、製作、復旧までの統合完了を意味しない。

## 実際の版差

同じ`common_native_probe`で、サバイバルとクリエイティブそれぞれ新しい実接続を使用する。
共通APIでbarrelを開き、stone 5をLeft PICKUPし、実source Emptyと実cursor stone 5を待つ。
外部RCONでプレイヤーを画面の有効範囲外へ移し、未改変vanilla自身に元の画面を閉じさせる。
調査中はClientのcloseを送らない。readonly proxyは元の圧縮frameをそのまま転送し、
元windowへの実CLOSEの到着と、その区間のClient close未送信を記録する。
trace ordinalとClientのreceive sequenceは別の番号である。

| 版 | survival / creativeの独立native結果 |
| --- | --- |
| 1.16.1 | stone 5のitem entityが現れ、在庫には元のdirt 2だけが残る |
| 1.21.11 | stone 5がhotbarへ戻り、元のdirt 2も残る。item entityはない |

元の公式JARの`AbstractContainerMenu.removed`もローカルで照合した。
legacyは非Empty cursorをdropしてEmptyへ変更する。
modernは生存中かつ切断していないServerPlayerならinventoryへ戻し、
死亡・切断等ではdropする。容量不足時のoverflowもあり得る。
このbytecode確認の分岐と、上の生存中／空き容量ありの実ネットワーク結果は区別する。
死亡・切断・容量不足のネットワーク実行はこの調査の検証範囲に含めない。
[InspectNativeClose.java](../scripts/InspectNativeClose.java)はJDKのjavapを呼ぶだけの自作wrapper。
Java 21の`--add-modules jdk.jdeps`で、SHA-1検証済みの元JAR／modern展開classpathへ実行する。
ゲームのJAR、mapping、bytecode本文、server.properties、worldはcrateへ配布しない。

強制closeは必ずしも新しいEmpty cursor／player UIの受信を発生させない。
次のケースには新しい実接続を使い、cursorをローカルでEmptyにしたりopenの検査を回避したりしない。
Clientの最後のcursor受信と、サーバーの在庫／item entity結果が一致すると仮定しない。

## 共通closeへ接続する順序

Clientは各版のclose後の処理へ任せず、画面終了前にプレイヤー在庫へ戻す。
constructorで検証済みのplayer slot対応を使い、同じstackの空き容量へ結合後、
空きmain／hotbarへLeft PICKUPする。storageへ返したりitemを生成したりしない。
容量を事前に検査し、dropを伴う暗黙のfallbackは行わない。

I/O前に元のsession／world／mode／opening／cursor／在庫と返却計画を保持する。
各返却stepの予測と、完全な送信、actual slot／cursor受信、legacy比較応答を別々に残す。
全stepの実受信と実Empty cursorを確認してからcloseを一度だけ送る。
vanillaの無応答をclose ACKへ変換しない。

この一連の処理をconnection-ownedの排他操作として保持し、待機側の取消後も再送しない。
途中の実値・mode・world・openingの競合は最初の理由を残し、値の復元では解除しない。
writerが止まっていても保存済みのintent／完了stepを確認できる入口が必要。
この調査の証拠は元のdisposal結果を保持する。後続の返却実装・検証は共通closeの資料へ分けて記載する。

## 同値再受信の修正

legacyのPICKUP／SWAP／QUICK_MOVEのowned taskは、準備と実I/Oの間で基準を再確認する。
Window Items等で内容が同じままordinalだけ更新された際にも、以前はwhole observed valueの比較で拒否していた。
修正後は既知の実受信値そのものを比較し、元のhistory ordinalは保存する。
送信直前のreceive sequenceを新しい`send.after_sequence`に設定し、
それ以前の再受信を結果として数えない。Submitted／Predicted／欠測は同値の実受信として扱わない。

決定的なtransport回帰テストでは、I/O前の保持済みSWAPへ実Set Slot／cursor packetを追加し、
一度の送信、historyの保持、新しい送信境界、両destinationの新しい実受信と比較応答を要求する。
別ケースでは実値を変更して復元し、I/O前に拒否され一つも送信されないことを確認する。
PICKUP／QUICK_MOVEも同じ修正を使用し、両mode／両版の既存native workflowを再実行する。

## 再現と証拠

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --accept-eula \
  --runtime-dir /dev/shm/voxrig-cursor-close-audit
```

Java 21を使い、serverは一つずつ、各JVM heapは1 GiB／CPUは1。
server停止を待ってから`.local/native-client-unification`へ結果を移し、tmpfsのworldを削除する。
source/dataのSHA-256をコンパイル前に取得し、consumer binaryのSHA-256と各reportへ保存する。
監査中のnative compression thresholdは通常の256で、変更／無効化しない。

配布する要約は[cursor close native evidence](../data/client_api/cursor_close_native_evidence.json)。
元のraw report／trace／stderr／server logは開発環境の`.local`に保持し、
要約にはそのSHA-256、actual Client record、独立RCON結果、元windowの実CLOSE frameを含める。
過去の失敗は成功で上書きしない。source snapshotがない古い試行を、後のsource hashへ結びつけない。
