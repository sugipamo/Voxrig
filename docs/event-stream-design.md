# 共通event streamの設計メモ

2026-10-07。実装前に方針を確認するためのメモ。

**決定（2026-10-07）**: 通知のみ・カーソル読み出し・下記12種類で実装した。
実装の結果、entityのeventは`EntityId`ではなく`native_id`を持つ形にした（理由は[共通のchange通知](common-events.md)）。

## 現状

- 1.16.1: `Bot::subscribe()`が`tokio::sync::broadcast`で67種類の`Event`を配る。
  多くの種類がpayloadを丸ごと持つ（`Event::Chat(ChatMessage)`、`Event::EntitySpawned(..)`等）。
  受信側が遅れると`Lagged(n)`で古いeventが失われ、何が失われたかは分からない。
- 1.21.11: eventを外へ流す仕組みはない。受信ループは各パケットに連番（`sequence`）を振り、
  状態を更新したあと`changed`通知を出す。
- 共通API: 観測（`capture`、`player_state`、`chat_after`等）はすべて受信連番を返す。
  待機API（`wait_for_*`）は「観測 → 判定 → 連番より新しい受信を待つ」で動く。

## 方針案

### 1. eventは「何が変わったか」の通知に絞り、中身は観測APIで読む（推奨）

```rust
pub struct ClientEvent {
    /// このeventを起こしたパケットの受信連番。観測の受信連番と同じ軸。
    pub receive_sequence: u64,
    pub kind: EventKind,
}
pub enum EventKind {
    BlocksChanged { min: [i32; 3], max: [i32; 3] },
    ChunkLoaded { x: i32, z: i32 },
    ChunkUnloaded { x: i32, z: i32 },
    InventoryChanged,
    ScreenChanged,
    PlayerChanged,        // 位置補正・体力・mode等
    WorldChanged,         // respawn・dimension変更・再設定
    EntitySpawned(EntityId),
    EntityRemoved(EntityId),
    ChatReceived,
    UiChanged,            // scoreboard・boss bar・title等
    Disconnected,
}
```

理由:
- 版ごとにpayloadの形が違う（JSONとNBT、item data等）。中身を共通化する作業は観測APIですでに済んでいる。
  eventにも同じ中身を持たせると、同じ変換を2か所で保守することになる。
- 通知を受けたら観測APIで読む、という一本の流れになる。観測は常に一貫した状態を返すので、
  「eventの中身」と「現在の状態」が食い違う問題が起きない。

代わりに、eventだけで状況を把握したい利用者にとっては1手間増える。

### 2. 配信はカーソルで読み出す方式にする（推奨）

chat履歴と同じく、各接続が直近N件（例: 4096件）のeventを保持し、利用者が読み出す。

```rust
let log = client.events_after(cursor).await?;        // すぐ返す
let log = client.wait_for_events(cursor, limit).await?; // 1件以上届くまで待つ
cursor = log.receive_sequence;
```

- 保持範囲より古いカーソルはエラーにする（`ErrorKind::State`、「この範囲は失われた」）。
  利用者は観測APIで現在の状態を取り直してから続ける。
  broadcastの`Lagged`と違い、どこからどこまで欠けたかが分かる。
- 遅い利用者がいても受信ループを止めない。複数の利用者が互いに影響しない。
- push型（`Stream`）が欲しい場合は、この上に薄い`Stream`アダプタを後から足せる。

### 3. 1.16.1の`Bot::subscribe` / `Event`はネイティブAPIとして残す

共通eventは`Client`側の新しいAPIで、1.16.1の`Event`を置き換えない。

## 実装の見積もり

- 共通の`EventLedger`（chatの台帳と同じ形）と`EventOps` trait: 小さい。
- 両版の受信ループで上記の種類を記録する: 中くらい。1.21.11は受信処理が分散しているので、
  各処理の入口で`EventKind`を記録する。
- 実サーバーでの確認: chat・待機と同じprobe方式で行う。

## 確認したいこと

1. eventの中身: 通知のみ（推奨）か、payloadも持たせるか。
2. 配信方式: カーソルで読み出す方式（推奨）か、push型の`Stream`を最初から用意するか。
3. 最初に対応する種類: 上の12種類で足りるか。追加・削除したいものはあるか。
