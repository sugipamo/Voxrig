# 版固有API（feature `native`）

版ごとの固有APIは、既定では無効の feature `native` の後ろにあります。

- `Client::native()`、`Client::java_1_16_1()`、`Client::java_1_21_11()`、`voxrig::Native`
- `voxrig::versions::java_1_16_1`（`Bot`、`BotManager` など）
- `voxrig::versions::java_1_21_11`（`NativeClient`、`operations()`、`checked_survival()` など）

既定のビルドでは、公開されるのは共通の `Client`（`voxrig::client::prelude`）だけです。固有APIは共通APIへ置き換えている途中で、置き換えが済んだものから削除していきます。

```toml
# 移行が終わるまでの一時的な指定
voxrig = { git = "https://github.com/sugipamo/Voxrig", branch = "main", features = ["native"] }
```

固有APIを使う example と integration test は `required-features = ["native"]` を持ちます。例えば `cargo run --features native --example multi_bot` のように実行します。

## 1.16.1 `Bot` から共通 `Client` への対応

| 1.16.1 `Bot` | 共通API |
| --- | --- |
| `player()` / `player_snapshot()` / `state()` | `client.player_state()` |
| `inventory()` / `inventory_snapshot()` / `open_window_snapshot()` | `player_state().inventory`、`client.screen_state()` |
| `entities()` / `observe_entities()` | `client.entities()` |
| entityの`metadata[index]` | `EntityObservation::data(EntityDataField::…)`（版ごとのindexと既定値を補う）、`angry(game_time)` |
| `survival_state()`の時刻 | `player_state().world_time` |
| `environment_state().eyes_submerged` | `client.eye_in_water()` |
| `block()` / `observe()` / `world_view_snapshot()` | `client.observe_region(region)`、`client.chunk([x, z])` |
| `loaded_chunks()` / `chunk_snapshot()` / `is_chunk_loaded()` | `client.loaded_chunks()`、`client.chunk([x, z])` |
| `raycast_blocks()` | `client.raycast_blocks(..)` |
| `set_control()` / `clear_control()` / `jump()` | `survival().start_control()` / `set_controls(..)` / `stop_control()` |
| `look()` / `select_hotbar()` | `survival().look(..)` / `select_hotbar(..)` |
| `use_item()` / `release_item()` / `swing_arm()` | `survival().use_item(hand)` / `release_use_item()` / `swing_arm(hand)` |
| `attack()` | `survival().attack_entity(..)` |
| 採掘（`start_digging` など） | `survival().dig(target, face)`、`dig_estimate(target)` |
| `place_block()` | `survival().use_on_block(..)`、`place_cube(..)` |
| `placement_info()` | `survival().placement_check(support, face)` |
| `craft_once()` / `compact_player_inventory()` | `survival().craft_once(..)` / `compact_inventory()` |
| `move_player_item()` | `survival().click_inventory(..)` / `transfer_inventory(..)` / `swap_hotbar(..)` |
| `close_window()` | `survival().close_container(screen)` |
| `respawn()` | `client.respawn()` |
| `revoke_connection()` | `client.revoke_connection()` |
| 接続状態 | `client.connection_status()`、`disconnect_reason()` |
| `subscribe()`（イベント） | `client.events_after(cursor)`、`wait_for_events(..)` |

`physics_metrics()` と、`environment_state()` の目の水中判定以外の値には、まだ共通の対応がありません。必要な値は `player_state()`（属性・effect・空気）から読めるものもあります。
