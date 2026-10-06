# 共通scoreboard観測とClientManager

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(config: ConnectionConfig) -> Result<()> {
    let manager = ClientManager::new(2)?;
    let client = manager.connect("worker", config).await?;
    client.wait_until_ready().await?;
    let board = client.scoreboard_state().await?;
    for score in board.scores {
        println!("{} {} {:?}", score.value.owner, score.value.value, score.source);
    }
    manager.shutdown().await?;
    Ok(())
}
```

## Scoreboard

両版の`Client::scoreboard_state()`は同じ`ScoreboardObservation`を返す。
objective・display-slot・score entryは、実受信の値と元のordinalを保持する。
各captureの`receive_sequence`と各値の受信時点は別で、`last_update_sequence`は削除やclearも含む。
初回packet未受信では`last_update_sequence=None`。空のcacheをserver側の全objectiveが空だと扱わない。
serverが表示中のobjectiveだけを送る場合があり、完全なserver catalogueではない。

表示方式は共通の`ScoreboardRenderType::{Integer, Hearts}`。
`UiText`は既存`ScreenTitle`と同じ型で、legacy JSON／modern unnamed NBTの元encodingを保持する。
modernのobjective/scoreにある任意のdisplayやnumber formatも保持する。
`ScoreNumberFormat`はblank・styledの元NBT・fixedの元componentを区別する。
component参照の解決、rendering、teams・boss bar・title等の全UIの共通化はまだ含めない。

decodeはpacket全体を検査してからcacheへ適用する。欠損や余分なfield、未対応number formatで
一部分だけを採用しない。新しいformatにはVoxrig更新が必要。cacheは4096entryへ制限する。
owner resetはobjective指定の有無を保持して適用し、objective削除は対応score・displayも除く。
legacy版固有UIでも、空objective名による全owner score削除を修正した。
原serializerとformat登録順序は[公式JARの確認記録](evidence/common-scoreboard-protocol-20261006.json)に固定する。

## Manager

`ClientManager::new(maximum_clients)`はactiveとpendingを合わせて1〜64を上限にする。
版やserverをmanager全体へ固定せず、`connect(name, config)`ごとにsetupする。
各Clientは独立したversion・live cache・registry所有情報を持ち、live cacheを共有しない。
異なる版のregistry IDを別Clientで解釈することは拒否する。

`connect`は名前とendpoint/profileをI/O前に予約する。
同じ名前、または同じhost/portと大文字小文字を無視したprofile名は別名のClientでも重複拒否する。
host別名や別processまで排他する機能ではない。接続取消・失敗で予約を解放し、pending transportも閉じる。
生成は認証済み接続であり、play readinessは`wait_until_ready()`で別に確認する。

`get(name)`は生成済みClientのcloneを返し、pendingはNone。
`names()`は生成済みの名前だけを返す。外部から切断されたClientも明示的な削除までは取得できるため、
取得結果をreadyの保証にしない。`disconnect(name)`で切断して削除できる。
pendingの個別disconnectはerrorで、接続callerの取消かshutdownを使う。

`shutdown()`は新規接続を止め、pending connectを取消し、実Clientを順番に閉じる。
外部に保持したcloneも同じ接続が閉じる。shutdownはterminalで、同じmanagerを再開しない。
途中でshutdown callerを取消した場合も未完了entryは保持し、明示的な次のshutdownで終了を続けられる。
単なるmanagerのdropはshutdownではない。event集約・自動reconnect・版固有metrics/cache共有は未統合。

この変更はA5のscoreboardとmanager部分。[かまど基本slot操作](common-furnaces.md)と
[乗車関係・明示的下車](common-vehicles.md)も両adapterへ接続し、両版のnativeで検証した。
A5の代表操作が揃った。より広いUI・vehicle・manager統合はBの必須作業に残る。
