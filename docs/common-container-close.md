# 共通Clientのコンテナclose

`survival.close_container(screen.id)`と`creative.close_container(screen.id)`は、1.16.1と1.21.11で同じ
`ContainerCloseRecord`を返す。`ScreenId`は現在の実OPENから取得する。mode一致・実empty cursor・未解決操作なしを要求し、
元のplayer/screenを同じ境界でI/O前に保持してcloseを一度だけ送る。unknown cursorやitem付きcursorでのdrop規則は後続作業。

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
| `Dispatched` | 完全なclose frameを書いた。サーバー側の閉鎖確認ではない |
| `ObservedClosed` | 完全送信に加え、元のopeningへの新しい実CLOSE packetを受信 |
| `RequiresInspection` | I/O前の画面/mode/cursor/worldの変化、送信不確実性等を保持 |

`server_close_sequence`は実CLOSEのordinalだけを保持する。送信成功、player WINDOW_ITEMS、別windowへのclose、
無関係な更新から生成しない。numeric IDを再利用した新しいOPENのcloseも旧openingへの応答にはしない。
未送信intentに実CLOSEが届いても、`dispatched == false`のまま成功へ変換しない。

送信済みのcloseは通常の次操作を妨げないが、元のScreenIdへのclose再送やstorageクリックは拒否する。
実際に新しいOPENを受信すれば、その新しいidentityから次の明示的操作を始める。
直前のrecordは次のcloseを開始するまで保持し、閉じた接続でも読める。
legacyではconnection-owned taskがactorのrevision/idle/exclusive-ownerを検査して一度だけ送信するため、waiter取消後も同じ送信が完了し得る。
modernではwriter待ちでの取消は未送信のintentとして残り、後から受信値が揃っても再送や成功へ変換しない。
送信途中の失敗はinspectionとし、不確実な接続を自動再利用しない。

`Client::screen_state()`は引き続き実受信の履歴である。close送信だけでscreenを消したり、active_windowを受信済み0へ変更したりしない。
legacyの互換UI cacheのみlocal closeとして消す。共通の受信screenとは別に扱う。
そのため最後に受信した画面が表示され続ける場合はclose recordも確認する。
close後のplayer画面の根拠は`player_screen: Some(SubmittedClose { close })`として別に返す。
同じ`swap_hotbar`による通常在庫の操作再開とactual player revisionの扱いは
[共通プレイヤー画面](common-player-screen.md)を参照。共通openのtarget geometry/結果照合は後続作業に残る。

公式未改変JARのcodecを[VerifyContainerClose.java](../scripts/VerifyContainerClose.java)で照合する。
[fixture](../data/client_api/container_close_packets.json)はlegacyのbyte ID 2件、modernのmulti-byte VarIntを含む3件。
Java 21、SHA-1検証済みの元JARとmodernの展開したnative server/libraries classpathを使い、versionの一致も検査する。
server/worldを起動せず、JAR・mapping・bytecodeはpackageへ配布しない。

共通consumerのfixtureでは両mode、元openingのactual reply、別windowのreply、numeric ID再利用、未送信取消とowned送信、
cursor拒否・未解決swapとの競合、actorの古いrevision/別owner拒否を検証する。
[native検証](common-client-native-validation.md)では同じconsumerがcreativeでcloseし、外部でchest内容を変更後に再OPENを実受信、
survivalでcloseして切断後の記録保持も確認する。独立RCONはcontents/player inventory/位置を照合するが、
RCONでmenu stateを取得できたとは主張しない。無応答をサーバーACKとして扱わない。
