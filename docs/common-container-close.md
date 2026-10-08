# 共通Clientのコンテナclose

`survival.close_container(screen.id)`と`creative.close_container(screen.id)`は、1.16.1と1.21.11で同じ
`ContainerCloseRecord`を返す。`ScreenId`は現在の実OPENから取得する。mode一致・既知の実cursor・未解決操作なしを要求し、
元のplayer/screenを同じ境界でI/O前に保持する。Empty cursorならcloseを一度だけ送る。
item付きcursorは、生存中の実health、constructorで検証済みのstorage画面、実player main/hotbar対応と十分な既知容量を要求する。
同じnative item種別/dataのstackへ結合した後、既知の空きmain/hotbarへLeft PICKUPで返却する。
通常itemの受信legacy NBT/modern componentsも保持し、元registryの意味と実効stack容量で計画する。
全返却stepの完全送信と実slot/cursor結果を待ち、実Emptyを確認してからcloseを一度だけ送る。
未知slotを空きと扱わず、容量不足時はintent保持・送信より前に失敗する。storageへの返却やdropのfallbackは行わない。
[元のnative調査](common-cursor-close-audit.md)で確認したlegacy drop／modern returnの差を、この共通手順で吸収する。
未解決constructor/data、data付き特殊item override、bundleの内部への収納は後続作業。
defaultのbundle Left空きslot移動など、既に検証済みの境界は保持する。

```rust,no_run
use voxrig::client::prelude::*;
# async fn close(client: &Client) -> Result<()> {
let screen = client.screen_state().await?.screen.expect("open screen");
let record = client.survival().close_container(screen.id).await?;
// Vanillaは通常closeをechoしない。送信と実応答は別の事実。
assert!(record.dispatched);
let latest = client.survival().container_close_record().await?;
# let _ = latest;
# Ok(())
# }
```

| stage | 意味 |
| --- | --- |
| `Pending` | I/O前のintent。取消されても自動retryしない |
| `ReturningCursor` | 元の排他closeが各PICKUPの実結果を待っている |
| `Dispatched` | 完全なclose frameを書いた。サーバー側の閉鎖確認ではない |
| `ObservedClosed` | 完全送信に加え、元のopeningへの新しい実CLOSE packetを受信 |
| `RequiresInspection` | I/O前の画面/mode/cursor/worldの変化、送信不確実性等を保持 |

`server_close_sequence`は実CLOSEのordinalだけを保持する。送信成功、player WINDOW_ITEMS、別windowへのclose、
無関係な更新から生成しない。numeric IDを再利用した新しいOPENのcloseも旧openingへの応答にはしない。
未送信intentに実CLOSEが届いても、`dispatched == false`のまま成功へ変換しない。

送信済みのcloseは通常の次操作を妨げないが、元のScreenIdへのclose再送やstorageクリックは拒否する。
実際に新しいOPENを受信すれば、その新しいidentityから次の明示的操作を始める。
直前のrecordは次のcloseを開始するまで保持し、閉じた接続でも読める。
両版ともconnection-owned taskが一連の返却からcloseまで通常操作を排他する。
waiter取消後も同じ処理は継続し、再送しない。`container_close_record()`はwriter待ち中も保持済みのrecordを返す。
`return_plan`はI/O前の返却先とPredicted値、`return_steps`は各PICKUPのactual before・送信境界・actual結果を保持する。
計画と実行stepで元registryを保持し、dataの正規化後も数量/native fieldが一致すれば続行する。
受信したconfiguration/tagの所有が変わった場合や、source/destination/無変更slotの意味が競合した場合は
inspectionとして保持する。予測を実受信へ昇格せず、各stepのactual source/cursorが必要。
modernのdata付きstepは、実受信revisionと別の実送信revisionで完全再同期を要求する。
Empty比較markerは実cursor hashではなく、再同期の要求だけで返却完了にはしない。
内部stepの`InventoryClickId::close()`は元の`ContainerCloseId`を返し、通常のclick履歴は上書きしない。
各stepは完全送信後5秒以内のactual source/cursor受信を要求し、legacyでは同じwindow/actionの実比較応答も要求する。
比較falseはnativeでclickが実行されてから比較が不一致だったことを表す場合があり、rollbackとは扱わない。
送信のwriter待ちはこの結果受信期限に含めない。返却の期限切れや途中のmode/world/opening/実値競合は
最初の理由と進捗をinspectionへ保持し、値の復元では解除しない。未解決ownerは他操作の許可に変換しない。

送信途中の失敗はinspectionとし、不確実な接続を自動再利用しない。

`Client::screen_state()`は引き続き実受信の履歴である。close送信だけでscreenを消したり、active_windowを受信済み0へ変更したりしない。
legacyの互換UI cacheのみlocal closeとして消す。共通の受信screenとは別に扱う。
そのため最後に受信した画面が表示され続ける場合はclose recordも確認する。
close後のplayer画面の根拠は`player_screen: Some(SubmittedClose { close })`として別に返す。
同じ`swap_hotbar`による通常在庫の操作再開とactual player revisionの扱いは
[共通プレイヤー画面](common-player-screen.md)を参照。共通openのtarget geometry/結果照合は[共通open](common-container-open.md)を参照。

公式未改変JARのcodecを[VerifyContainerClose.java](../scripts/VerifyContainerClose.java)で照合する。
[fixture](../data/client_api/container_close_packets.json)はlegacyのbyte ID 2件、modernのmulti-byte VarIntを含む3件。
Java 21、SHA-1検証済みの元JARとmodernの展開したnative server/libraries classpathを使い、versionの一致も検査する。
server/worldを起動せず、JAR・mapping・bytecodeはpackageへ配布しない。

共通consumerのfixtureでは両mode、元openingのactual reply、別windowのreply、numeric ID再利用、未送信取消とowned送信、
未知cursor・容量不足・未解決swapとの競合、actorの古いrevision/別owner拒否を検証する。
[native検証](common-client-native-validation.md)では同じconsumerがcreativeでcloseし、外部でchest内容を変更後に再OPENを実受信、
survivalでcloseして切断後の記録保持も確認する。独立RCONはcontents/player inventory/位置を照合するが、
RCONでmenu stateを取得できたとは主張しない。無応答をサーバーACKとして扱わない。

返却のdefault-item Left PICKUPは元の未改変JARで53,070ケース、4,956 packet codec往復を実行した。
Legacy default constructorが持つ`Damage=0` NBTはそのまま保持し、空きslotへの移動を認める。
modern default bundleは空きcursor/空きslotとのLeft移動だけを対象とし、非Empty同士の内部収納は除外する。
primitiveの再現・source/classpath/output hashは[exporter](../scripts/export_cursor_returns.py)と
[source](../data/client_api/cursor_return_source.json)にある。これだけでnetwork/mode/owner/close完了を証明したとは扱わない。

実networkの返却・closeは[返却native evidence](../data/client_api/cursor_return_native_evidence.json)へ保存した。
両modeのstone 5はmainのstone 63へ1個結合し、別の空きmainへ4個返す。default helmet 1も両版で返却し、
modernではdefault bundle 1も両modeで返却する。独立RCONでexact count、dropなし、空になって閉じたbarrel、位置・向き不変を確認した。
途中で発見した同値pose ordinalの誤拒否と、modern OPENがteleport確認を追い越す競合のfailed runも残す。
最後の両runは同じconsumer binaryを使用し、JVM exit 0・runtime削除済み。
記録したbuild後の変更はpackage/docs、frozen-evidence試験とRust field shorthandのlint整理であり、actual runtime input hashは書き換えない。
