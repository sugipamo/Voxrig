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
`creative().land()`は直前のowned flight stepの位置を使い、既知の乾いた床と立位の空間を検査する。
先に`move_flying`で床へ戻し、`land`を呼ぶ。自動降下や経路選択は利用側へ残す。

`land`は飛行解除とneutral inputを送信し、明示的なzero controller seedから2つのreleased ground
model ticksをClient所有の処理で送る。最初のtickはground false、床との下向きcollisionを処理した
次のtickはground trueとなる。`FlightLanding`に宣言したseed、解除／neutralの送信、
有限motionの予測・attempted/dispatched tickを保持する。未来のnative physics全体やserverの停止ACKは作らない。
正常なstanding姿勢、通常attribute、受信effectsなし、未解決impulseなし、完全なdry supportと
1/16の水平planning reserveを要求する。空中・欠測・未対応形状ではI/O前に拒否する。

受信abilitiesのflying bitが残る場合も、同じ受信ordinalより後に完全送信した明示的解除を
local modelの根拠として扱う。実受信flagsを変更しない。後続のground runはこの根拠を引き継ぐ。
後から同じflying flagsを受信してもordinalが変われば、その古い解除根拠を使わない。
姿勢・velocity・effect・world・pose correction等の既存guardも引き続き検査する。
着地後の`preview_path`／`start_predicted_path`、block targeting・収納等は同じCreative handleから継続する。

## 検証

両adapterの軽量TCP試験はwriterを停止させ、待機を取り消した後も元recordを読み取れること、
writerを再開すると元frameが一回だけ送られること、次のstepに別attemptが付くことを確認する。

`creative-flight` nativeシナリオは、同じcommon consumerでdry slab/stairを上り、収納を開閉した後、
同じClientからflightを要求し、3つの短い飛行位置を送る。RCONはbaseline後に読み取りだけで使い、
serverの最後の位置とflight flagを独立確認する。proxyの元packetと保持intentを照合し、
飛行解除frameも一回だけ送る。終了後の地上継続や飛行物理全体の同等性はこの試験では確認しない。

飛行送信の固定commit `3a254a6`では、1.16.1の`trial-1.16.1-597eb958`、
1.21.11の`trial-1.21.11-127651d9`が成功した。
同じ422 source/data入力と同じbinaryを使い、両JVM exit 0、proxy errorなし。
詳細は[入力と実サーバー証拠](evidence/common-creative-flight-20261006.json)を参照。

同commitの単体721件・公開API2件・doctest29件（別環境等の8件ignored）と、Clippy warnings denied、
Rust 1.85 all-target、rustdoc warnings denied、fmt、proxy境界4件、配布allowlistと
755 filesの実Cargo package buildを確認した。重いcheckとserverはすべて順番に実行した。


`creative-landing`はさらに床へ戻るstep→owned `land`→27tickの新しい地上移動→
fresh chest開閉→切断まで同じClientで通す。
1.16.1 `trial-1.16.1-547b9037`／1.21.11 `trial-1.21.11-001bd50b`が成功し、
同じ422入力とbinary、両JVM exit 0、proxy errorなし。
独立nativeの着地位置・flying false・後続移動終点と、実OPENの新しいscreen／ordinalを確認した。
詳細は[着地と継続の入力・結果](evidence/common-creative-landing-20261006.json)を参照。

着地追加後は単体725件・公開API2件・doctest29件が成功（専用環境等の8件ignored）。
Clippy warnings denied、Rust 1.85 all-target、rustdoc warnings denied、fmt、trace境界4件と、
開発directoryを除く756 filesのCargo package buildも成功した。
全体試験後のClippy指摘はcfg(test) moduleの配置を修正して再確認した。
