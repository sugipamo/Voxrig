# Java 1.16.1 の掘削ACK訂正と結果

1.16.1 の ACK (`0x07`) は position、block state、status、successful を持つ。
サーバーから受信した状態は成功・拒否にかかわらず world に適用する。
この適用と同じ coherent-state 境界で、共通 `events_after` に次を順番に記録する。

1. `BlocksChanged { min: position, max: position }`
2. `LegacyDiggingAcknowledged { position, block_state_id, status, successful }`

後者は元の受信事実であり、最新状態の別キャッシュではない。status は native の
0 START / 1 ABORT / 2 FINISH。状態IDは1.16.1専用。`successful=false` はサーバーの
拒否を表す。`true` でも空気への変化・自分による除去・継続安全性は保証しない。
操作を追跡していない利用者や、transaction を drop した利用者にも通知する。
同値の状態、拒否の直後の成功、遅いACKも省略・合流しない。

訂正は既存のBody証拠・transaction・common miningに加え、pending placementの
support / target 判定にも届く。既存のtransactionの `Rejected` と、common miningの
`LegacyReply { accepted: false, .. }` / `RequiresInspection` を保つ。
後続の成功・air受信は既にラッチした拒否を消さない。送信の事前拒否は増やしていない。
この変更は受信訂正の公開であり、受信前に上位が持っていた仮説の一致を要求しない。

未ロードchunkはACKで生成しない。この場合も位置の無効化とACK原値を公開するが、
`chunk` / `capture` の現在値は未観測のままであり、空気と推定してはならない。

## 利用例

Golemkitなどの上位は、接続ごとのcursorと地形から導出した計画を持つ。
通知の位置に依存する経路・採掘計画を破棄し、現在値を再観測してから再計画する。

```rust,no_run
use std::time::Duration;
use voxrig::client::prelude::*;

async fn watch(client: &Client) -> voxrig::Result<()> {
    let mut cursor = 0;
    loop {
        // State エラーも呼出元へ返す。欠落を「変化なし」に置き換えない。
        let log = client.wait_for_events(cursor, Duration::from_secs(30)).await?;
        for event in log.events {
            match event.kind {
                EventKind::BlocksChanged { min, max } => {
                    // この範囲に依存する古い計画を捨て、最新の受信地形を再読する。
                    let _current = client.capture(voxrig::Region { min, max }).await?;
                }
                EventKind::LegacyDiggingAcknowledged { position, status, successful, .. } => {
                    // position/statusは受信時の対象/action。命令の一意なIDではない。
                    println!("{position:?}: action={status}, accepted={successful}");
                }
                EventKind::WorldChanged => { /* 前worldの派生値を破棄する */ }
                EventKind::Disconnected => return Ok(()),
                _ => {}
            }
        }
        cursor = log.cursor;
    }
}
```

`EventLog::cursor` は通知の通し番号。`receive_sequence` と混同しない。
同じpacketの2通知は同じ `receive_sequence` を持つ。独立した読者はcursorを共有しない。
最新4096通知を保持する既存の有界ログを利用し、古いcursorは `ErrorKind::State`。
`wait_for_events` も同じエラーを返す。欠落後は計画と履歴完全性を撤回する。
現在の地形は再読できるが、失ったACKの成否・命令対応は再読では復元できない。
利用可能な新しいcursorを持たない場合は接続を作り直して観測を再確立する。
切断後も保持中の受信事実を読める。`WorldChanged` / `Disconnected` は同じ順序ログ上の
境界なので、過去の位置・ACKを現在worldの操作権限として再利用しない。

## 情報の所有

- 重複して乖離していたのは、サーバーのブロック、Voxrigの受信worldと、上位が持つ対象・
  地形・経路の派生状態。ACKの訂正が共通通知から欠落していた。
- 正はサーバーから受信したACKの状態とsuccessful。現在値の所有者はVoxrigのworld reducer。
  eventは受信事実、MiningRecordはその操作の診断結果として区別する。
- ACK適用時に対象の旧world状態を置き換え、通知を読んだ上位は対象に依存する派生計画を
  捨てる。world境界・切断・ログ欠落でもそのscopeの前提を撤回する。
- 対象の最新状態の再読はO(1)、経路の再計画費用は上位次第。ACKの元の成否・順序は
  最新地形から再導出不能。有界eventログへの記録はACKごとにO(1)、追加通知は2件。

## 互換性と検証

`EventKind` は既に `#[non_exhaustive]` なのでvariant追加はadditive。
既存のAPI署名と結果型を変更しない。Java 1.21.11のprocessing ACKはsequence進捗であり、
block stateや成功boolを持たないため、この通知には写さない。1.21.11固有の変更はない。

fixture: `digging_ack_notification_tests` と common mining / placement の
`digging_ack_*`。全actionの成否、未対応ACK、未ロード領域、world境界、切断後の保持、
履歴のevictionエラー、拒否のラッチとplacement訂正を検証する。

既存の `run_climbing_control` ハーネスで公式1.16.1 JARのSHA-1を確認し、RCONで独立に
サーバー状態を照合する。元packetを変更しないproxyでACK原値と共通eventを比較する。

```sh
cargo build --locked --features native --example digging_ack_probe
python3 -B scripts/run_digging_ack_live.py --accept-eula \
  --binary target/debug/examples/digging_ack_probe \
  --jars /path/to/official-jars --output .local/ns02/live
```

証跡: [公式1.16.1 LIVE](evidence/digging-ack-live-1.16.1-20261027.json.gz)。
