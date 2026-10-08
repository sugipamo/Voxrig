# 共通の待機API

受信状態が条件を満たすまで、上限時間付きで待つ。1.16.1・1.21.11の両方で使える。

| メソッド | 返る条件 | 戻り値 |
| --- | --- | --- |
| `wait_for_receive(after, limit)` | 受信連番が`after`より大きいパケットが適用された | 新しい受信連番 |
| `wait_for_block(position, limit, accept)` | そのセルの状態が`accept`を満たす（未ロードは`None`） | 満たした時点の`Capture` |
| `wait_for_loaded(region, limit)` | 領域の全セルがロード済み（airもロード済み） | 満たした時点の`Capture` |
| `wait_for_chat(cursor, limit)` | `cursor`より後にchatが1件以上届いた | `ChatLog` |

- 各待機は「観測 → 条件判定 → その観測の受信連番より新しいパケットを待つ」を繰り返す。
  受信連番で比較するため、判定と待機の間に起きた変化を取りこぼさない。
- 上限時間を過ぎると`ErrorKind::Timeout`、接続が閉じると`ErrorKind::Disconnected`を返す（両版共通）。
- 条件を満たしたことはclientが受信した状態であり、サーバー上の確定ではない。
- 受信連番は接続ごとの通し番号で、server tickではない。別の接続の連番とは比較できない。

内部では`WaitOps::wait_for_receive`を両版が実装する。1.16.1はパケット適用ごとの通知を、
1.21.11は既存の`changed`通知を使う。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、`examples/wait_probe.rs`で確認した。
consoleから`setblock`と`tellraw`を送って外部の変化を起こした。`setblock`は待機開始の約1秒後に投入したので、
検出までの遅れは約0.1秒以内。

| 版 | `wait_for_loaded` | 満たされない条件 | `setblock`の検出 | `wait_for_chat` | 切断後 |
| --- | --- | --- | --- | --- | --- |
| 1.16.1 | 27セル | `Timeout` | 待機開始から約1.08秒 | 1件 | `Disconnected` |
| 1.21.11 | 27セル | `Timeout` | 待機開始から約1.08秒 | 1件 | `Disconnected` |
