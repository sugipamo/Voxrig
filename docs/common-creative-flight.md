# 共通Creative飛行の送信と保持

`Client::creative().set_flying`／`move_flying`は、受信Creative mode・flight permissionと
現在の接続を検査し、一回の送信intentを保持してClient所有の処理へ渡す。
`move_flying`は現在のlocal位置から最大4blockの送信であり、経路選択・collision解決・
vanillaの飛行物理・serverによる位置受理を保証しない。

地上の有限移動が完了したClientからも飛行できる。飛行を有効にするframeが完全に送られた時点で、
以前の地上移動結果を診断履歴へ退避する。次の飛行stepへ古い立位検査を持ち込まず、
以前の地上終点を飛行後の立位根拠として再利用もしない。

呼び出し側が待機を取り消しても、保持した送信処理はClientが所有する。
`Client::flight_record()`で最新の`FlightRecord`を同期的に取得できるため、writerやadapterの
state lockが詰まっていても、そのlockを待たずに元intentを確認できる。
`Prepared`、完全送信の`Submitted`、失敗の`RequiresInspection`を区別する。
未解決のintentは次の共通mutationを拒否し、失敗や取消から自動再送しない。
保存されたJSONからrecordや実行許可は復元できない。

`FlightRecord::initial`はclaim直前の観測、`command`は要求値、`dispatched`は完全なframe送信を表す。
現在位置はSubmittedとして更新するが、実受信pose・velocity・abilitiesへ送信値を上書きしない。
`set_flying(false)`は飛行停止の要求だけであり、停止した立位や着地ACKを作らない。
明示的な着地から同じClientで地上操作を継続する契約は、ロードマップB3の後続作業である。

## 検証

両adapterの軽量TCP試験はwriterを停止させ、待機を取り消した後も元recordを読み取れること、
writerを再開すると元frameが一回だけ送られること、次のstepに別attemptが付くことを確認する。

`creative-flight` nativeシナリオは、同じcommon consumerでdry slab/stairを上り、収納を開閉した後、
同じClientからflightを要求し、3つの短い飛行位置を送る。RCONはbaseline後に読み取りだけで使い、
serverの最後の位置とflight flagを独立確認する。proxyの元packetと保持intentを照合し、
飛行解除frameも一回だけ送る。終了後の地上継続や飛行物理全体の同等性はこの試験では確認しない。

1.16.1の`trial-1.16.1-597eb958`、1.21.11の`trial-1.21.11-127651d9`が成功した。
同じ422 source/data入力と同じbinaryを使い、両JVM exit 0、proxy errorなし。
詳細は[入力と実サーバー証拠](evidence/common-creative-flight-20261006.json)を参照。

単体721件・公開API2件・doctest29件（別環境等の8件ignored）と、Clippy warnings denied、
Rust 1.85 all-target、rustdoc warnings denied、fmt、proxy境界4件、配布allowlistと
755 filesの実Cargo package buildを確認した。重いcheckとserverはすべて順番に実行した。
