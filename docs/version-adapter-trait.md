# 共通trait設計と版別対応表

2026-10-07。`integration`ブランチ（`codex/client-api-unification`の先端＝mainの全成果を含む）を
基準にした、版間API差を埋めるための設計正本。互換性は維持しない。

## ブランチ

| ブランチ | 役割 |
| --- | --- |
| `main` | 変更しない（保全） |
| `integration` | 既存成果の合流点。`codex/client-api-unification`と同一commitから作成。他の作業ブランチはすべてmainへ統合済み |
| 作業ブランチ（`claude/…`） | `integration`から分岐し、破壊的変更を伴う整理を行う |

リモートに`develop`は存在しない。

## 現状の層構造

```
Client (enum Adapter { Java1_16_1(Box<Bot>), Java1_21_11(Bot) })
 ├─ 共通API: connection.rs, client/** が各版の common_* を呼ぶ
 ├─ Survival / Creative ハンドル: モード検査付きの同じ共通API
 ├─ 版固有の出口: java_1_21_11_operations(), observe_client_region(), checked() …
 └─ 旧1.16.1 API: crate rootへの glob 再export（Bot, BotManager, 約200型）
```

問題点:

1. 共通契約が「両版に同名の`common_*`メソッドがある」という暗黙の一致でしか表現されておらず、
   `Client`側に約150の手書き`match`が重複していた。
2. crate rootが1.16.1専用の型で埋まっており、利用者が共通APIと旧APIを区別できない。
3. 版固有の機能が`Client`の通常メソッドとして混在し、`Unsupported`を返すだけの分岐が多い。
4. `Support::Restricted`の説明文が長大で、契約の要点が読み取れない。

## 共通trait

### 第1段階（実装済み: `src/client/adapter.rs`）

`pub(crate) trait VersionAdapter`に共通操作を一覧で宣言し、`version_adapter!`マクロで
両版の`Bot`へ実装する。シグネチャが一致しなければコンパイルエラーになる。
`Client`は`dispatch!(&self.adapter, a => VersionAdapter::method(a, …).await)`で選択中の版へ委譲する。

```rust
pub(crate) trait VersionAdapter {
    const VERSION: MinecraftVersion;
    fn connection_id(&self) -> u64;
    async fn wait_until_ready(&self) -> Result<()>;
    async fn disconnect(&self) -> Result<()>;
    async fn player_state(&self) -> Result<PlayerObservation>;
    async fn capture(&self, region: Region) -> Result<Capture>;
    async fn execute(&self, mode: GameMode, action: Action<'_>) -> Result<Option<i32>>;
    // … 全59メソッド。一覧は adapter.rs が正本
}
```

trait objectは使わない（`async fn`を`dyn`にするとbox化とSend境界が必要になるため）。
版の数は少なく固定なのでenumと静的dispatchで十分。

### 第2段階（目標形）

1. **関心ごとに分割する。** `VersionAdapter`を次の部分traitの合成にする。
   `Session`（接続・準備・切断・記録）、`WorldView`（capture・region）、`PlayerView`（自身・inventory・respawn）、
   `EntityView`、`UiView`（scoreboard・boss bar・teams・titles・tab・border）、`InventoryOps`、
   `CraftingOps`、`BlockOps`（target・place・mine）、`MotionOps`（path・flight）、`VehicleOps`。
2. **実装本体をtrait implへ移す。** 現在は各版の`common_*`へ転送しているだけ。
   転送層を消し、`common_`接頭辞の重複名を無くす。
3. **版固有機能は`Client::native()`へ集約する。**
   ```rust
   pub enum Native<'a> { Java1_16_1(&'a java_1_16_1::Bot), Java1_21_11(java_1_21_11::Native) }
   ```
   `java_1_21_11_operations()`、`observe_client_region()`、`observe_shared_client_region()`、
   `Survival::checked()`はここへ移す。共通APIは`Unsupported`分岐を持たない。
4. **crate rootを共通APIにする。** `voxrig::prelude`は`client::prelude`。
   旧`Bot`/`BotManager`は`voxrig::versions::java_1_16_1`からのみ参照する。
5. **能力表を構造化する。** `Support::Restricted(&str)`の長文を、短い要約と
   docsへのリンクに置き換える。厳密な前提条件は実装の検査とdocsに残し、型の文字列には持たせない。

### 共通化の基準

- 両版でゲーム上の意味が同じで、入出力を共通型で損失なく表せるものだけをtraitに入れる。
- 片方の版でしか実装していないものは、もう片方を実装してからtraitへ昇格させる。
  `Unsupported`を返す実装をtraitに入れない。
- 版依存のID・packet形式・物理定数は各版のモジュールに閉じ込める。

## 対応表

凡例: ◎ 共通trait経由で両版対応 / ○ 両版にネイティブ実装はあるが共通化されていない /
▲ 1.16.1のみ / △ 1.21.11のみ / × どちらにもない

### 接続・セッション

| 機能 | 状態 | 共通API | 1.16.1 native | 1.21.11 native |
| --- | --- | --- | --- | --- |
| 接続・準備・切断 | ◎ | `connect` `wait_until_ready` `disconnect` | `Bot::connect` 等 | `Bot::connect` 等 |
| 接続の即時無効化 | ◎ | `revoke_connection` | `revoke_connection` | session fence |
| 受信packet記録・再生 | ◎ | `start/stop_packet_trace` `connect_recorded` | `common_recording` | `PacketTrace` |
| 接続ID・login identity | ◎ | `connection_identity` | | |
| 複数client管理 | ◎ | `ClientManager` | `BotManager` | |
| event stream | ▲ | — | `subscribe` (`Event`) | なし |
| 接続状態・generation・操作admission | ▲ | — | `connection_state` `admit_operation` | なし |
| client settings・resource pack・brand | ▲ | — | `set_client_settings` `respond_resource_pack` `server_brand` | resource packは拒否 |

### 世界の観測

| 機能 | 状態 | 共通API | 1.16.1 native | 1.21.11 native |
| --- | --- | --- | --- | --- |
| 領域のblock観測 | ◎ | `observe_region` `capture` | `observe_region_snapshot` | `observe_region` |
| registry（静的・server受信） | ◎ | `registry` `server_registry_state` | | |
| block検索・geometry query | ▲ | — | `query_loaded_blocks` `query_geometry` `capture_loaded_geometry` | なし |
| 一貫観測（light・時間・entity同時） | ▲ | — | `capture_coherent_observation` | light配列は検証のみで保持しない |
| chunk待機・block変化待機 | ▲ | — | `wait_for_chunk(s)` `wait_for_block_*` | なし |
| ピストン・隣接更新の再構成 | △ | — | なし | `observe_client_region` |
| block NBT・entity NBT query | ▲ | — | `query_block_nbt` `query_entity_nbt` | なし |
| map・tag・command tree | ▲ | — | `maps_snapshot` `tags_snapshot` `command_tree_snapshot` | なし |
| raycast（block・entity） | ○ | `target_block`（静的outlineのみ） | `raycast_blocks` `raycast_entities` `can_see_*` | `observe_player_target` |

### 自身の状態

| 機能 | 状態 | 共通API | 1.16.1 native | 1.21.11 native |
| --- | --- | --- | --- | --- |
| 位置・体力・mode・inventory | ◎ | `player_state` `received_inventory` | | |
| respawn | ◎ | `respawn` | `respawn` | |
| 属性・effect・経験値 | ○ | — | `survival_state` | `LocalPlayerState` |
| advancement・統計・recipe book | ○ | `received_recipes` のみ | `advancements_snapshot` `statistics_snapshot` | recipe受信のみ |

### entity

| 機能 | 状態 | 共通API | 1.16.1 native | 1.21.11 native |
| --- | --- | --- | --- | --- |
| spawn/despawn一覧 | ◎ | `entity_spawns` | | |
| 受信motion | ◎ | `entity_motion` | | |
| metadata・装備・hitbox・体力 | ▲ | — | `entities` `entity` (`EntityState`) | なし |
| 他playerの観測 | ○ | — | `entities` | `visible_players` |
| interact・attack | ◎ | `Survival::interact_entity` `attack_entity` | | |
| 位置指定interact | ▲ | — | `interact_entity_at` | なし |

### UI

| 機能 | 状態 | 共通API |
| --- | --- | --- |
| scoreboard・boss bar・teams・player list・titles・tab list・world border | ◎ | `scoreboard_state` `boss_bars` `teams` `player_list` `titles` `tab_list` `world_border` |
| chat受信 | ○ | — （1.16.1 `Event::Chat`、1.21.11 `system_messages_after`） |
| chat送信 | ▲ | — （1.16.1 `send_chat`。1.21.11は`send_command`のみ） |
| command送信 | ○ | — （両版に`send_command`） |
| tab補完 | ▲ | — |

### inventory・container・crafting

| 機能 | 状態 | 共通API | 1.16.1のみの機能 |
| --- | --- | --- | --- |
| 通常click・shift移動・hotbar交換 | ◎ | `click_inventory` `transfer_inventory` `swap_hotbar` `swap_container_hotbar` | 任意mode・任意slotの`click_slot` `dispatch_window_clicks` |
| container開閉・画面観測 | ◎ | `open_container` `close_container` `screen_state` | |
| crafting（recipe配置・結果取得） | ◎ | `place_recipe` `take_crafting_result` `transfer_crafting_result` | `craft_once` |
| かまど | ◎ | `FurnaceObservation` | |
| 装備・持ち替え・drop | ▲ | — | `dispatch_equip` `swap_hands` `drop_selected` |
| 村人取引・エンチャント台・ビーコン・金床 | ▲ | — | `select_trade` `select_enchantment` `set_beacon_effects` `rename_item` |
| 本・看板編集 | ▲ | — | `edit_book` `update_sign` |
| creativeでのitem生成 | ◎ | `Creative::set_hotbar` | |

### blockと移動

| 機能 | 状態 | 共通API | 1.16.1 native | 1.21.11 native |
| --- | --- | --- | --- | --- |
| look・hotbar選択 | ◎ | `look` `select_hotbar` | | |
| 採掘 | ◎ | `start_mining` `finish_mining` `abort_mining` | `dig_block`（汎用） | 限定的な`start_survival_mining` |
| 設置 | ◎ | `place_cube` | `place_block`（汎用） | 限定的な`place_survival_cube` |
| creativeの破壊・使用 | ◎ | `Creative::break_block` `use_on_block` | | |
| item使用（空中） | ▲ | — | `use_item` `release_item` `swing_arm` | なし |
| 歩行・ジャンプ（有限入力列） | ◎ | `preview_path` `start_predicted_path` | 汎用物理 | 乾いた地形のみ |
| 自由な継続入力（`set_control`） | ▲ | — | `set_control` `jump` `clear_control` | なし |
| 液体・梯子・登攀など汎用物理 | ▲ | — | `physics.rs` | なし |
| creative飛行 | ◎ | `set_flying` `move_flying` `land` | | |
| 乗り物 | ◎ | `dismount` `vehicle_control` `resume_ground` | `steer_boat` `set_vehicle_control` | |

## 差を埋める優先順位

利用者が汎用botを書くときに困る順。

1. **event streamと待機API**: chunk・block・inventoryの変化待ちとeventを共通化する。
   1.21.11はreceive loopの`sequence`と`Notify`があるので、待機は比較的容易。
2. **chat送受信・command**: 両版にほぼ実装があり、共通化だけで済む。
3. **entityの現在状態**（metadata・装備・hitbox・他player）: 1.21.11で受信しているが捨てているpacketが多い。
4. **block検索・geometry query・raycast**: 共通の`Capture`上に版非依存で実装できる。
5. **汎用物理（`set_control`）**: 1.21.11の物理を乾いた地形以外へ広げる。最も大きい作業。
6. **特殊container**（取引・エンチャント・金床など）と編集系。

## 簡素化の方針

厳密な契約は「場合により簡素化してよい」とする。

- 残す: 世代・切断・送信の不確実性の区別（誤った再送を防ぐ）、未ロードをairと扱わないこと、
  受信値と予測値の区別。
- 簡素化する: 1.21.11専用の`checked_survival`系（`ObservedDryCubeV1`、`PredictedDryCubeV1`、
  採掘のretirement・profile recoveryなど）。これらは共通API（`Survival`）と機能が重なっているため、
  共通APIで足りる部分を削り、残りは`native()`配下の実験的APIへ移す。
- `Support::Restricted`の長文を要約にする。

## 作業ブランチでの手順

1. ✅ `VersionAdapter` traitと`dispatch!`の導入（挙動は不変、全test通過）。
2. ✅ crate rootの整理: 1.16.1型のglob再exportを外し、`voxrig::prelude`を共通APIへ切り替える。
3. 版固有機能を`Client::native()`へ移す。
4. `VersionAdapter`の分割と、転送層の除去。
5. 上記の優先順位で差分を埋める。
