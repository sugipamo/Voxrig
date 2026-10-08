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
| `EntityUpdated { native_id }` | `EntityUpdated`（移動・向き・速度・metadata・装備） | `SYNC_ENTITY_POSITION`、`REL_ENTITY_MOVE`、`ENTITY_MOVE_LOOK`、`ENTITY_LOOK`、`ENTITY_HEAD_ROTATION`、`ENTITY_METADATA`、`ENTITY_VELOCITY`、`ENTITY_EQUIPMENT`、`ENTITY_TELEPORT` |
| `EntityStatus { native_id, status }` | `EntityStatus` | `ENTITY_STATUS`（codeは版ごとに違う） |
| `EntityDamaged { native_id }` | `EntityStatus`のうち被害のcode（2・33・36・37・44） | `DAMAGE_EVENT` |
| `PlayerKilled { native_id }` | 自分の死亡（combat eventのdeath） | `DEATH_COMBAT_EVENT` |

- `BlocksChanged`の範囲は実際の変更を含む上位集合。1.21.11のmulti-block変更はsection（16×16×16）単位になる。
- entityのeventは`EntityId`ではなく`native_id`を持つ。操作に使う`EntityId`は`entity_spawns()`で対応する
  ものを引く（`EntityId`はworld単位の識別を含み、event記録時点では安全に作れないため）。
- 各eventの`received_after`は、接続のevent logを作ってからそのeventを記録する（パケットを適用する）までの
  時間（clientの時計）。移動の標本の到着時刻などに使う。
- 1.16.1の`Bot::subscribe()` / `Event`はネイティブAPIとしてそのまま残る。

## 死亡の文言・切断の理由・接続状態

- `Client::death_message()`: 自分の最後の死亡の文言（1.16.1はnativeのJSON、1.21.11はNBTの`UiText`）と受信連番。
  次の死亡まで残る。切断後も読める。
- `Client::disconnect_reason()`: serverがkickで送った文言。切断後も読める。
- `Client::connection_status()`: `Joining`（login・設定・最初の同期）、`Ready`、`Closing`、`Closed`、`Unknown`
  （送受信の結果が分からない終わり方）。
- 1.16.1は待機中も毎tick位置を送る。serverがkickの直後に接続を閉じると、この送信が先に失敗して状態は`Unknown`になる。
  そのときも、すでに届いているframeを最大200 ms読み、kickの文言だけを記録する（ほかのパケットは適用しない）。
  文言は状態が変わった少し後に入る。読む前に送信の失敗で受信bufferが破棄された場合は残らない。
- 1.21.11は待機中に送らないので、kickは`Closed`と文言になる。
- 2026-10-08に両版の公式serverで確認した: 豚のteleportで`EntityUpdated`、即時ダメージの効果で
  `EntityDamaged`、`kill`で自分の`PlayerKilled`と死亡の文言、`kick`で文言（1.16.1は`Unknown`、1.21.11は`Closed`）を
  受け取った（`examples/entity_events_probe.rs`）。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、`examples/event_probe.rs`で確認した。
consoleから`setblock`・`fill`・`summon`・`kill`・`tellraw`・`title`・`give`を送った。

| 版 | 届いた種類 | `setblock`位置を含む`BlocksChanged` | 受信連番の順序 | 切断後 |
| --- | --- | --- | --- | --- |
| 1.16.1 | BlocksChanged、ChunkLoaded、EntitySpawned/Removed、ChatReceived、InventoryChanged、PlayerChanged、UiChanged | `[214,7,-227]..[218,8,-225]` | 昇順 | `Disconnected` |
| 1.21.11 | BlocksChanged、ChunkLoaded、EntitySpawned/Removed、ChatReceived、InventoryChanged、UiChanged | `[-16,-64,0]..[-1,-49,15]`（section） | 昇順 | `Disconnected` |

同じblockを置き直すとサーバーは変更を送らないため、確認ではいったんairにしてから置いた。
