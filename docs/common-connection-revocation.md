# 共通Clientの緊急遮断

`Client::revoke_connection()`は1.16.1と1.21.11で同じ同期APIを使う。
観測capture・writer取得・送信終了・通常のdisconnect cleanupを待たず、
元transportをローカルで不可逆に遮断する。同じClientの全cloneが同じ遮断を共有する。
繰り返し呼んでも同じ`ConnectionRevocation`を返す。呼出し時にTokio runtimeへ入っている必要はない。

```rust,no_run
use voxrig::client::prelude::*;
fn quarantine(client: &Client) -> ConnectionRevocation {
    client.revoke_connection()
}
```

返り値は版とprocess-local connection IDを持つローカルの事実で、world generationや
受信ordinalを後から取得・合成しない。実受信`SessionStamp`の版／connection IDと照合できる。
Serializeは診断のために使い、Deserializeや操作guardを復元する入口は持たない。
この値から新しいClient、画面・entity・操作ID、復旧permissionを作らない。

正常なlogout、transportのclose完了、送信の取消成功、server上での静止を証明するAPIではない。
既にwriterへ入ったframeはprefixだけ届いた可能性があり、結果を配送不明のまま保持する。
writer取得前の待機は新しいframeを送らずに終了する。遮断後は元接続を再利用・再接続しない。
新しいClientや明示的な採掘復旧は、それぞれの新しい接続・受信条件と一度性を検査する。
通常の`disconnect().await`はcleanupやwriter shutdownを待つため、同じ契約ではない。

旧版はmainで追加されたnative generation revocationを使用する。
新版は既存の送信停止とpartial-write guardを維持し、受信taskをabortしてwriter shutdownを
接続時に保持したruntimeへ予約する。送信待ちの処理も停止通知で終了し、writerの解放を待たない。
診断recordを成功へ変更したり、pendingな操作を消去したりしない。

## 検証

共通consumerの同じ同期呼出しを両adapterへ通す。観測captureとwriterを同時に保持したまま、
別のOS threadから遮断し、cloneの同一結果と32個の送信待ちの拒否を確認する。
新版では受信taskがロック待ちのままabortされること、1byteで停止したframeのprefixと
`UncertainDispatch`が残ること、別接続には送信できることも検査する。
旧版のnative partial-write／lost ACK／別generationの分離試験も回帰検査に含む。

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario connection-revocation --accept-eula
```

同じconsumerで実Clientへ接続し、通常のhotbar選択を一度送ってから遮断する。
元Clientとcloneの追加選択を拒否し、両handleを保持したままnative playerの不在とproxyのEOFを
独立観測する。RCONはfixture準備後には読み取りだけを使う。
この試行で実際に観測したcloseを、すべての緊急遮断におけるtransport終了保証とは扱わない。
成功runは1.16.1 `trial-1.16.1-5a4b4868`／1.21.11 `trial-1.21.11-cf6a85c5`。
同じ427 source/data入力とconsumer binaryを使い、両JVM exit 0・proxy errorなし。
[入力と結果](evidence/common-connection-revocation-20261006.json)を参照。
全体の単体734件・公開API2件・doctest30件も成功した（専用環境等の8件ignored）。
fmt、all-target Clippy、Rust 1.85 all-target check、rustdoc warnings denied、
trace 4件、配布allowlistと766 fileのCargo package buildも成功した。
広いcontext／記録／再構成／復旧と、停止writer下での各履歴取得経路はB5の残作業である。
