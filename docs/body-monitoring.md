# Body の遅延訂正監視（Java 1.16.1）

`Body::action_observation(id)` は、操作結果と後追い監視の完全性を別々に返す。
`HeadlessBody` の同名メソッドも利用できる。送信や再試行は行わない。

- `acceptance`: 当時の結果。terminal になった後は不変。Accepted は当時の
  有界な受理方針を満たしたという意味であり、永続的なサーバー保証ではない。
- `outcome`: 観測できた最初の後発 Corrected、または当時の結果。
- `monitoring`: 証拠をどこまで確認できたか。操作を Rejected に変更しない。

`BodyBackend` の新メソッドは既定で Unsupported。Java 1.21.11 の
`HeadlessBody` も新メソッドに Unsupported を返す。既存の同版処理は変更しない。

| monitoring | 意味・上位の処理 |
|---|---|
| NotMonitoring | Pending、拒否、未送信など、terminal Accepted/Unknown の送信済み監視対象でない。完全性を主張しない |
| CompleteThrough { cursor } | baseline から cursor まで欠落なく調べた。未来・サーバーtick・最新サーバー状態の保証ではない |
| Gap { after, retained_from } | after 後から retained_from までの証拠が失われた。依存する「訂正なし」の仮定を破棄 |
| WorldChanged { after, world } | 元の world 寿命を追跡できない。旧 world の計画を破棄 |
| ChunkChanged { after, at, chunk } | 対象chunkの置換/unload。旧対象の追跡を破棄。at は元の証拠ordinal |
| TemporarilyUnavailable { after, reason } | 今回は timeout/取得エラー。after は維持。再照会可能で、訂正がなかったと解釈しない |
| CorrectionObserved { at } | 最初の訂正を outcome に保存し、その証拠境界で監視終了 |

Gap / WorldChanged / ChunkChanged は最初の失効境界を action の保持期間中保存し、
その後の最新値や world への復帰で CompleteThrough に戻さない。欠落後に新しい
block値が届いても、欠けた途中の rollback は復元できない。取得失敗のみ再試行し、
成功した走査の後に cursor を進める。訂正・失効境界を走査順に評価し、失効した
chunk の後に届いた値を同じ寿命の訂正として採用しない。world が既に変わった
snapshot や gap がある場合には、先に完全性を撤回する。

## 有界の変更ログ

`Body::monitor_changes_after(MonitorCursor::default(), maximum)` は全保持対象を
更新して、action ID と監視状態の変化を返す。`maximum` は 1..=4096。
普通の `submit` と `send_and_trust` の両方を対象とする。新しい受信のない
同じ状態の再照会はイベントを増やさない。訂正・失効の通知は一度だけ記録する。

monitor log は BodyConfig.retention 件の独立したリングで、同じ設定値が
通常の BodyEvent と action の保持上限にも使われる。`MonitorCursor` は
通常の `EventCursor` とは別型であり、cursor は返した範囲までだけ進む。
monitor log の eviction は `MonitorChanges.gap=true`。外側の
`BodyEvents.gap` と、内部証拠の `MonitoringStatus::Gap` は別物。
将来cursor、別backend、範囲外maximumは InvalidInput。
古いaction IDや別backend IDの照会も明示的な InvalidInput。

status/events/monitorの読取時に更新する方式であり、バックグラウンド通知ではない。
1.16.1 の既存 `events_after` と send-and-trust の `action_status` /
`action_outcome` も新しい監視経路を通り、失効を monitor log に書く。
普通の `submit` の `action_status` は従来どおり不変の当時の結果を返す。
監視のためには新APIを購読する。ログ自体の欠落時には保持されているIDを
`action_observation` で読み直す。失効情報はIDに残るが、期限切れIDや欠落した
途中履歴を復元できるとは扱わない。再送を回復手段にしない。

```rust,no_run
# async fn inspect<B: voxrig::body::BodyBackend>(body: &voxrig::body::Body<B>, id: voxrig::body::ActionId) -> voxrig::Result<()> {
use voxrig::body::{MonitorCursor, MonitoringStatus};
let observed = body.action_observation(id).await?;
match observed.monitoring {
    MonitoringStatus::Gap { .. }
    | MonitoringStatus::WorldChanged { .. }
    | MonitoringStatus::ChunkChanged { .. } => { /* 依存する計画を破棄 */ }
    MonitoringStatus::TemporarilyUnavailable { .. } => { /* 今回未確認。後で再照会 */ }
    _ => {}
}
let changes = body.monitor_changes_after(MonitorCursor::default(), 256).await?;
if changes.gap { /* 保持IDを再照会し、失われた履歴への依存を撤回 */ }
let next = changes.cursor;
# let _ = next;
# Ok(()) }
```

## 情報所有とコスト

1. 重複して乖離していたのは受信側 EvidenceLog の保存範囲、HeadlessBody の
   Monitor/過去の Acceptance、上位の「訂正を追跡できている」という認識。
2. 正はサーバーから届いた元の証拠と受信側の保持範囲。Acceptance は当時の
   結果の所有者に残し、監視の完全性を別軸にする。上位はcursorと派生計画を持つ。
3. gap、world、対象chunk寿命の最初の失効時に追跡の完全性と依存計画を捨てる。
   一時的取得失敗は今回未確認。当時の結果を失効や拒否で上書きしない。
4. 元の結果/監視状態の保持は action ごと O(1)（既存の訂正snapshotを除く）。
   証拠は4096件、変更ログとactionはretention件に有界。更新は保持action数×
   未処理証拠数と選択blockの読取に比例する。成功後にcursorを進めるため、
   読取済みの無関係な証拠のevictionだけでは失効しない。現在値の再観測は
   O(選択領域)。欠落した途中rollbackの完全な再導出は不可能。

## 検証

1.16.1 fixture は内部4096件のeviction、取得timeout/エラーと回復、world/chunk
境界、Unknownの理由保持、普通のsubmitの後発訂正、並行読取と重複抑制、
monitor log eviction/ページング/cursor検査を確認する。
公式1.16.1 LIVEは `scripts/run_body_monitor_live.py`。既存の
`run_climbing_control` ハーネスでSHA-1確認済み公式サーバー、RCON、元packet
プロキシを使い、当時Acceptedの保持、実teleport訂正、内部証拠gap、world変更、
専用ログの欠落/ページングを確認する。timeout/取得エラーはfixtureで決定的に確認。

2026-10-27 の [LIVE証跡](evidence/body-monitor-live-1.16.1-20261027.json.gz)
は全4チェックが pass、trace error は0件。公式server SHA-1、probe binaryと
関連sourceのSHA-256、RCONと元packetの証拠を含む。

[4構成の検証結果](evidence/body-monitor-validation-20261027.json) に、実行コマンド、
終了コード、テスト集計、5つのfixture結果、ログとソースのhashを保存する。
