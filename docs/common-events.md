# 共通のchange通知（event）

「何が変わったか」を受信連番付きで通知する。中身は観測API（`capture`、`player_state`、
`received_inventory`、`chat_after`、`entity_spawns`等）で読む。1.16.1・1.21.11の両方で使える。
設計の経緯は[設計メモ](event-stream-design.md)を参照。

```rust
let mut cursor = 0;
loop {
    let log = client.wait_for_events(cursor, Duration::from_secs(30)).await?;
    for event in &log.events {
        match event.kind {
            EventKind::BlocksChanged { min, max } => { /* capture(Region { min, max }) */ }
            EventKind::Disconnected => return Ok(()),
            _ => {}
        }
    }
    cursor = log.cursor;
}
```

## 読み出し

- `events_after(cursor)`: `cursor`より後のeventを古い順に返す。最初は`0`、以後は返り値の`cursor`を渡す。
  `cursor`はeventの通し番号で、受信連番ではない（1パケットから複数のeventが出るため）。
- `wait_for_events(cursor, limit)`: 1件以上届くまで待つ。上限を過ぎると`ErrorKind::Timeout`。
- 各eventの`receive_sequence`は原因になったパケットの受信連番で、観測APIの受信連番と同じ軸。
  返り値の`receive_sequence`は、そのlogがどの受信連番まで完全かを示す。
- 接続ごとに最新4096件を保持する。保持範囲より古い`cursor`は`ErrorKind::State`で失敗する。
  その場合は観測APIで現在の状態を取り直してから、新しい`cursor`で続ける。
- 切断後も読める。最後のeventは`Disconnected`で、その後は何も記録されない。
- `EventKind`は`#[non_exhaustive]`。種類は今後増える。

## 種類と元のパケット

| `EventKind` | 1.16.1（既存`Event`からの対応） | 1.21.11（パケット） |
| --- | --- | --- |
| `BlocksChanged { min, max }` | `BlockChanged`、`BlockEntityUpdated`、multi-block変更（変更位置の範囲） | `BLOCK_CHANGE`、`TILE_ENTITY_DATA`、`MULTI_BLOCK_CHANGE`（section全体の範囲） |
| `ChunkLoaded` / `ChunkUnloaded` | `ChunkLoaded` / `ChunkUnloaded` | `MAP_CHUNK` / `UNLOAD_CHUNK` |
| `InventoryChanged` | `InventoryUpdated`、`SlotUpdated`、`HeldItemChanged` | window 0の`WINDOW_ITEMS`/`SET_SLOT`、`SET_PLAYER_INVENTORY`、`SET_CURSOR_ITEM`、`HELD_ITEM_SLOT` |
| `ScreenChanged` | `WindowOpened`、`WindowClosed`、`WindowProperty`、`MerchantOffers` | それ以外のwindowの`WINDOW_ITEMS`/`SET_SLOT`、`OPEN_WINDOW`、`CLOSE_WINDOW`、`CRAFT_PROGRESS_BAR` |
| `PlayerChanged` | `Position`、`PositionCorrection`、`Vitals`、`Experience`、`GameStateChange`、`SurvivalStateUpdated` | `POSITION`、`UPDATE_HEALTH`、`EXPERIENCE`、`ABILITIES`、`GAME_STATE_CHANGE` |
| `WorldChanged` | `Login`、`Spawn`、`Respawn` | `LOGIN`、`RESPAWN`、`START_CONFIGURATION` |
| `EntitySpawned { native_id }` / `EntityRemoved { native_id }` | `EntitySpawned` / `EntitiesDestroyed` | `SPAWN_ENTITY` / `ENTITY_DESTROY` |
| `ChatReceived` | `Chat` | `SYSTEM_CHAT`、`PLAYER_CHAT`、`PROFILELESS_CHAT` |
| `UiChanged` | `UiStateUpdated`、`PlayerListUpdated` | scoreboard・boss bar・teams・player info・title・tab list・world borderの各パケット |
| `Disconnected` | `Disconnected`、接続エラー | 切断を検知した最初の読み出し時 |

- `BlocksChanged`の範囲は実際の変更を含む上位集合。1.21.11のmulti-block変更はsection（16×16×16）単位になる。
- entityのeventは`EntityId`ではなく`native_id`を持つ。操作に使う`EntityId`は`entity_spawns()`で対応する
  ものを引く（`EntityId`はworld単位の識別を含み、event記録時点では安全に作れないため）。
- 1.16.1の`Bot::subscribe()` / `Event`はネイティブAPIとしてそのまま残る。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、`examples/event_probe.rs`で確認した。
consoleから`setblock`・`fill`・`summon`・`kill`・`tellraw`・`title`・`give`を送った。

| 版 | 届いた種類 | `setblock`位置を含む`BlocksChanged` | 受信連番の順序 | 切断後 |
| --- | --- | --- | --- | --- |
| 1.16.1 | BlocksChanged、ChunkLoaded、EntitySpawned/Removed、ChatReceived、InventoryChanged、PlayerChanged、UiChanged | `[214,7,-227]..[218,8,-225]` | 昇順 | `Disconnected` |
| 1.21.11 | BlocksChanged、ChunkLoaded、EntitySpawned/Removed、ChatReceived、InventoryChanged、UiChanged | `[-16,-64,0]..[-1,-49,15]`（section） | 昇順 | `Disconnected` |

同じblockを置き直すとサーバーは変更を送らないため、確認ではいったんairにしてから置いた。
