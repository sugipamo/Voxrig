# 選択中stackのdrop（1.21.11）

`survival.drop_selected(SelectedDropAmount::One)` / `creative.drop_selected(...)`は、
選択中の実受信stackから1個または全量（`All`）を一度だけ要求する。
1.16.1の共通入口・読取は明示`Unsupported`。既存native APIの契約は変えない。
任意countの連続送信・自動再送・rollbackはない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn drop_one(client: &Client) -> Result<()> {
let intent = client.survival().drop_selected(SelectedDropAmount::One).await?;
let latest = client.survival().selected_drop_record().await?;
// Pending / RequiresInspection / Expiredでは再送せず、同じrecordを読む。
# let _ = (intent, latest);
# Ok(())
# }
```

`SelectedDropRecord`は元session/world、mode、selection（canonical slot 36..44）、
countとitem dataを含む`main_before`、cursor、期待する減少`expected_after`を同一境界で保持する。
実受信player画面か同一openingへの完全送信済みclose、実受信stack/cursor、
実受信または完全送信済みselection、handleに一致するSurvival/Creativeと競合ownerがないことを要求する。
空・欠測・不正count・容量超過・未解決item constructor・既知のitem使用中はI/O前に拒否する。
受信欠測を空stackや未使用へ補完しない。既知の非空cursorは保持できる。
詳細なitem data比較と画面条件は[在庫交換](common-inventory-swaps.md)と
[共通プレイヤー画面](common-player-screen.md)と同じ規則に従う。

版別adapterが`PLAYER_ACTION`のDROP_ITEM=4 / DROP_ALL_ITEMS=3、ZERO/DOWNをencodeし、
sequenceはdig/use/place/主手交換と同じ接続全体のcounterから確保する。
原版dropは減少後のslotをclient既知として登録するため、通常のslot通知を省略する。
そのまま受信結果を捏造せず、drop frameの後にplayer画面0・slot -1・Left PICKUP・revision -1の
no-op resyncを一度だけ送る。原版revisionは0..32767なので不一致となり、full contents/cursorが返る。
slot -999の画面外dropとは異なり、slot -1ではitem/cursorを動かさないことを原版menuで検査する。
これは追加のdropや任意slotクリック列ではなく、1回の操作に付随する実受信の要求である。
`send.dispatched`はdrop frame、`send.resync_dispatched`はno-op frameの完全送信であり、
どちらも在庫結果を生成しない。どちらかの送信中に取消・部分writeがあればinspectionに保持する。
`Pending`から`ObservedDecreased`になるには、両frameの完全送信と、選択slotの実更新ordinalが
`send.after_sequence`を超え、元のitem dataのまま1個減少、または期待どおりEmptyになった受信が必要。
無関係なslot・片側だけの競合・block ACK・拾得entityの出現や回収通知・cacheの一致は代用しない。
これは受信した減少の照合であり、自分のdropによる変化や他actorからの因果ACKを保証しない。
別の消費が同じ減少として届いた場合をprotocol上で区別することはできない。

両frameの完全送信後、resyncのfull player contents（46 slotとcursorが`send.after_sequence`より
新しい同じ受信ordinal）で選択slotとcursorが送信前と同じなら、終端の`ObservedUnchanged`になる。
`unchanged_boundary`にそのordinalを保持し、`main_receipt`は記録しない。
原版はPLAYER_ACTIONを後続のresync clickより先に処理するので、このfull contentsはdrop処理後の境界である。
同じ値の単一SET_SLOTや送信前の受信は境界にならない。原版`handlePlayerAction`は、spectatorと、
死亡してrespawn待ちなど`hasClientLoaded()`がfalseのplayerのdropを何も送らずに無視する。
spectatorへの変更はmode変更として先に届くため`RequiresInspection`になる。
`ObservedUnchanged`は終端で、inventory ownerを解放してledgerを完了し、後の受信で書き換えない。

record保持から5秒（inventory executorと同じ）以内に`Pending`を抜けない場合は`Expired`になる。
結果は不明でserverが後から適用しうる（拒否の証明ではない）。ledgerは`TransactionExpired`で失敗し、
その接続では以後の操作を受け付けないので再接続する。`Expired`も後の受信や切断で書き換えない。
新しいstageはどちらも`#[non_exhaustive]`な`SelectedDropStage`への追加である。

取消・部分write・切断・world/configuration変更・選択/mode/screen/cursor変更、
既知のitem使用や期待と異なる消費・data変更は最初の理由を`RequiresInspection`に保持する。
後から値が復元しても解除せず、新しいdropやclickへ進めない。
未解決recordは既存inventory ownerと同じ排他とledgerへ接続し、drainはslot結果を待つ。
完了済みrecordは後の在庫変化や切断から独立した履歴として凍結する。
次の明示的なdropだけが最新captureから新しいIDを作る。

構成packet試験は両mode、1個・最終1個・全量、禁止mode、空stack、古い/未受信値、
item data保持と不一致、競合と復元、取消・部分write、両方向の排他、ledger、
entity通知非ACK、full contentsとcursor競合を検査する。
close後はplayer画面の実full resyncで完了できることを検査する。
未完了中の実configuration移行・切断、drop完全送信後のresync部分writeは、
元record・inspection・失敗ledgerを保持することを検査する。
`scripts/export_selected_drop.py` / `ExportSelectedDrop.java`は未変更公式1.21.11 codecで
両actionのsequence境界10件とresync frameを往復し、`data/client_api/selected_drop_*`を生成する。
既存の`ExportRegularClicks.java`の観測contextを使い、native menu全slotとcursorが
no-op前後で不変である3ケース（cursor 0/1/7個）を別のprimitive事実として確認する。
観測wrapperだけをnamed packageへcompileし、署名済みnative classは変更しない。

```bash
python3 -B scripts/export_selected_drop.py --server-home /absolute/path/1.21.11 \
  --runtime-output .local/selected-drop-codec
cargo build --locked --example climbing_control_probe -j2
python3 -B scripts/run_selected_drop_live.py --accept-eula \
  --server-jar /absolute/path/1.21.11/server.jar --output /absolute/path/evidence
```

ライブharnessは指定JARの一時copyだけを起動し、world/EULAも一時directoryへ作る。
両modeの1個・最終1個・全量、fresh slot受信・data保持・元packetの一度性と、
独立RCONのSelectedItem/地面itemの個数とdataを照合する。全processを終了時に停止する。
codec・構成packet・liveの根拠はそれぞれ別の検証として扱う。

不変・期限切れの構成packet試験は、両量のfull contentsによる`ObservedUnchanged`とowner解放・
後の減少で書き換えないこと、単一slotの不変更新では終端しないこと、cursor不一致のfull contents、
応答なしの`Expired`と`TransactionExpired`のdrainを検査する。
2026-10-27に公式1.21.11で、keep_inventoryで死亡しrespawn前のplayerの1個dropが
`ObservedUnchanged`（RCONでも12個のまま）になり、respawn後の同じ接続のdropが`ObservedDecreased`に
なることを`scripts/run_drop_handswap_no_effect_live.py`で確認した
（[実行根拠](evidence/drop-handswap-no-effect-live-1.21.11-20261027.json.gz)）。
既存の`run_selected_drop_live.py`の6ケースも再実行して成功した。

2026-10-22の全6ケースは成功し、公式JAR・40classpath入力・SDK/consumerのhash、
元packetとfresh slot・RCONの対応を[実行根拠](evidence/common-selected-drop-20261022.json)に保存した。
最初のdrop単独試験でslot通知の省略を確認し、no-op resync追加後に全ケースを再実行した。
