# 利用クライアント向け共通APIの網羅表

2026-10-09にmain `77c74d9`の公開入口・`Feature::ALL`・native入口・既存検証資料を照合した。
Golemkit、DustRoute、minetool、deepplanningなど、Voxrigを利用するアプリのための一覧。
同じサーバー上の別playerの観測もentity／profileの行に含む。
利用プロジェクトのソースや依存固定先は変更していない。

対象はJava 1.16.1／1.21.11のprotocol-facingな身体・sensor。
「共通入口がある」「限定した操作ができる」「公式serverの特定場面を通した」を区別する。
この一覧を作ったことは、残る機能の実装完了や非公開利用側の移行完了を意味しない。

## 導入と実行時の契約

通常入口は`voxrig::client::prelude::*`。接続時に版を固定し、対応するregistryから名前とpropertyを解決する。
native整数ID、保存したJSON、別接続のcursorから現在の操作対象を作らない。
`Capabilities::for_version(version)`／`client.capabilities()`は静的な実装情報で、
準備完了・現在のgame mode・権限・chunkの存在を保証しない。操作ごとの検査が必要。

接続せずに全52機能群の現在の宣言をJSONとして取得できる。

```sh
cargo run --locked --example client_capabilities > /tmp/voxrig-capabilities.json
```

出力は既存の`Support`のserde表現と制約文書のパスを使う。
`feature`はDebug表示による一覧ラベルで、永続的なprotocol IDではない。
`Feature::ALL`にないゲーム機能を対応済みと判断しない。

利用側が守る共通の条件:

- `wait_until_ready`の後にもworld／mode／元spawn／元screenの寿命を確認する。
  所有関係が変わったら古い対象を捨て、観測し直す。
- `ObservedValue.source`と各fieldの受信連番を確認する。capture全体の連番が進んでも、
  個々のfieldが新しく受信されたことにはならない。欠測と明示的な空・falseを区別する。
- 送信receipt、client予測、実受信結果を区別する。位置の送信、neutral入力、attackの送信は
  serverの受理、停止、命中を証明しない。
- 取消・部分送信・timeout後は残ったrecordを調べる。未解決操作を自動再送しない。
  `revoke_connection`で送信権限を遮断し、必要な観測・履歴を保存する。
- change通知は4096件、entity履歴は8192件（1read最大1024件）、chatは256件。
  event overflowでは再観測、entity履歴では明示的な`gap`を処理する。
  履歴の時刻はSDK適用時計でありsocket到着・server tick・UTCではない。
  読取時計と有限tailは[PR #35](https://github.com/sugipamo/Voxrig/pull/35)で追加済み（main `6db149b`）。
  それ以前を固定している利用側は採用commitを確認する。
- 未対応item data、metadata、geometryを既定値・air・空stackへ置き換えない。
  同じitem名だけでstackを結合しない。

接続設定・利用側で確認する順序は[公開API](public-client-api.md)、
[移行手順](client-api-migration.md)、[利用側検証](client-api-consumer-validation.md)を参照する。

## 現在の共通機能: 全52群

下の各Featureは`Feature::ALL`に対応する。両版で共通入口があり、厳密な制約は
[能力表](common-capabilities.md)と各機能文書で確認する。
nativeの機能数やpacket分岐数を共通機能の数へ加算しない。

| 利用操作 | Feature | 公開入口と制約 |
| --- | --- | --- |
| 接続の遮断・復帰 | `ConnectionRevocation`, `Respawn` | `revoke_connection`、死亡受信後のowned `respawn`。自動再接続なし |
| 名前・ID・受信registry | `Registry` | `registry`、`server_registry_state`。版／接続／configurationの所有情報を保持 |
| 自分と周囲の一括観測 | `WorldObservation`, `PlayerObservation` | `capture`、`observe_region`、`player_state`。受信とlocal model、欠測を区別 |
| playerの受信文脈 | `PlayerContext` | `player_context`。能力・飛行／歩行速度・難易度・経験値・天候・default spawn・view。元source、world所有、欠測と既知のゼロ／falseを保持 |
| chunkの受信文脈 | `ChunkContext` | `chunk_context`。biome volume・heightmap・block entity NBT。元sourceと列のincarnationを保持し、地形から補完しない |
| 地図の受信 | `Maps` | `map_observation`。header・optional icons・部分pixelと各source。世界ごとのID、欠測pixel、bounded cacheとimmutable capture |
| 視点・hotbar | `BasicControls` | `look`、`select_hotbar`、`player_control(mode)`。Spectatorのhotbarは拒否 |
| block検索・ray・照準 | `BlockQueries`, `BlockTargeting`, `SurvivalTargeting` | `find_blocks`、64block collision ray、handleのstatic outline targeting。形状・姿勢の制約は別 |
| 読取専用予測 | `SurvivalPreview` | `preview_path`。1〜120tick、限定したdry地形・状態 |
| 有限地上移動 | `SurvivalMovement`, `CreativeMovement` | `start_predicted_path`、`motion_record`。予測の完了はserverの受理ではない |
| 継続移動 | `ContinuousControl` | `start_control`／`set_controls`／`request_ground_jump`／`stop_control`。液体、梯子、つる、足場、playerの泡の柱。接地ジャンプはsession所有の一度だけの予約と結果記録。飛行・乗車は別 |
| Creative操作 | `CreativeControls` | default stack、有限飛行・着地。広い姿勢／effectからの継続は残る |
| 採掘・明示復旧 | `Digging`, `SurvivalMining`, `SurvivalMiningRecovery` | `dig`／`dig_estimate`とretained mining。既定tool速度、受信effect／属性。復旧は同profileの限定条件 |
| 設置・使用 | `SurvivalPlacement`, `PlacementCheck`, `ItemUse` | receipt付き`place_cube`、読取専用`placement_check`、送信のみの`use_item`／`use_on_block`／解除 |
| inventory・装備の転送 | `InventorySwap`, `InventoryClick`, `InventoryTransfer`, `InventoryHelpers` | `received_inventory`、通常PICKUP／QUICK_MOVE／SWAP、cursor収納・merge。全click mode対応ではない |
| container・かまど | `ContainerObservation`, `Containers` | `screen_state`、通常storage／table／furnaceの開閉とconstructor由来layout。特殊screenは残る |
| 製作 | `Crafting` | received recipe／grid、結果取得・転送、owned recipe配置。材料計画は利用側 |
| 別player・mobの現在状態 | `EntityObservation`, `EntityMotion`, `EntityState` | `entity_spawns`、`entity_motion`、`entities`。元spawn寿命、field別sample、health／equipment／named data。boxは型の既定寸法 |
| entity受信履歴 | `EntityHistory` | `entity_history_after`。spawn／motion／status／animation／remove／world変更／自身補正。metadata／equipment履歴は残る |
| entityの指定操作 | `EntityInteraction` | `interact_entity`／`attack_entity`。旧版Dragon部位は明示的な派生モデル。命中・reach証明や自動戦術なし |
| 乗車・操縦・下車 | `VehicleObservation`, `VehicleInput`, `VehicleDismount`, `VehicleGrounding` | `vehicle_state`、有限`start_vehicle_control`、owned下車、限定した地上継続。ボートの水面／水没／流水／泡、元の速度受信の一度だけの反映、server駆動トロッコ |
| chat・command | `Chat` | unsigned `send_chat`／`send_command`、`chat_after`。署名必須serverは対象外 |
| change通知と待機 | `Events`, `Waits` | cursor通知、receive／block／loaded／chat／event待機。entity payload履歴とは別 |
| profiles・team | `PlayerList`, `Teams` | `player_list`、`teams`。tab登録は空間上のentityの存在ではない |
| UIの受信 | `DisplayObservation`, `BossBars`, `Scoreboard` | title／action bar／tab text／world border／boss bar／scoreboard。描画・補間・完全server catalogueの推定なし |
| 記録・再生・scene | `PacketRecording`, `PacketReplay`, `SurvivalScene`, `RecordingAndReconstruction` | bounded raw trace、選択状態の読取専用replay、immutable dry scene。saved dataから実行権限を復元しない |
| 複数接続 | `ClientManagement` | `ClientManager`、1〜64 named Clients。取消・shutdown、版／registry分離。自動再接続やevent集約なし |

chunk列と受信光は[common-chunks](common-chunks.md)、dynamic registryは
[common-server-registries](common-server-registries.md)に契約がある。
これらは独立のFeature名を持たず、上記のworld／registry観測から読む。

## 共通化の残件

利用側の具体的な移行阻害項目を先に扱う。継続操作の停止／再開で古い速度を再適用し向きを0へ戻す
[Issue #37](https://github.com/sugipamo/Voxrig/issues/37)は[PR #50](https://github.com/sugipamo/Voxrig/pull/50)で修正済み。
単発接地ジャンプの[Issue #49](https://github.com/sugipamo/Voxrig/issues/49)は共通sessionが予約・物理・取消と結果記録を持つ。
armor／offhandの厳密なPICKUP／交換は[Issue #38](https://github.com/sugipamo/Voxrig/issues/38)のP1。
読取時計の[Issue #36](https://github.com/sugipamo/Voxrig/issues/36)はPR #35の統合で解決済み。

優先度は実装順の提案。P1は誤った利用判断を防ぐ観測と在庫、P2は用途別の拡張、
P3は現在のoffline二版以外の導入条件。各項目内でも場面を分けて実装・検証する。
以下のIssue一覧は網羅のための追跡であり、すべてを次のPRで一括実装する宣言ではない。

| ID・優先度 | 利用側に必要な残件 | 現状の根拠と受入条件 |
| --- | --- | --- |
| OBS-ENTITY / P1 | pose／子供／scaleを反映したentity box・eye、未解読metadata、metadata／equipment履歴、modern相対補正・特殊トロッコsample | [entity現在状態](common-entities.md)・[motion](common-entity-motion.md)・[history](common-entity-history.md)。既定boxと受信確定値を分離し、未確定なら理由を返す。remove／ID再利用／world resetと両版実接続を検査 |
| OBS-WORLD / P1 | さらに未監査のworld/player文脈 | [`Client::player_context()`](common-player-context.md)で能力・難易度・経験値・独立天候field・default world spawn・world-view fieldを提供。[`Client::chunk_context()`](common-chunk-context.md)で受信biome、heightmap、block entity NBTと列のincarnationを提供。[`Client::map_observation()`](common-maps.md)で地図のheader、icon、部分pixelを両版へ提供。元の受信sourceとworld generation、未受信と既知のゼロ／falseを保持。追加の文脈とmodern decode監査は#41継続 |
| INVENTORY / P1 | 明示slot装備・drop・hand swap、armor／offhand／resultのclick残差、製作台入力SWAP／QUICK_MOVE、複雑item/componentの残constructor・比較・hash | [item data](common-item-data.md)・[inventory](common-inventory-clicks.md)・[recipes](common-recipes.md)。容量・item data・cursor／slot保存を検査し、拒否／取消／部分I/Oで再送しない。装備計画は利用側 |
| WINDOWS / P2 | villager trade、enchant、anvil、beacon、brewingなど特殊menuとsign／book編集 | nativeには一部入口があるが共通layout・結果契約は未完。openingに束縛した対象、slot／property／選択の元値、消費と出力の受信を保持。別screen／mode／close／取消を検査 |
| MOVEMENT / P2 | ボートのentity衝突・特殊block hook、残る装備移動効果、別姿勢／飛行／elytra／他の乗り物 | [vehicles](common-vehicles.md)・[control](common-control.md)・[physics](physics-engine.md)。ボート／トロッコを先行する。公式処理oracle＋元送信packet＋独立server pose、neutral／強制下車／補正後の停止を検査 |
| INTERACTION / P2 | 姿勢・属性に合うreach／eye、entity ray／visibilityと位置指定interaction、非cube／waterloggedなどの確認付き設置と結果待機 | `use_on_block`は汎用送信として既存。確認付き操作への拡張を混同しない。未知shape／動くblock／欠測／変更競合を拒否し、対象blockと材料の実受信を別々に保持 |
| PROTOCOL / P2 | sound／stop sound、particle、world／block action、pickup、break progress、explosion文脈、camera／leash、command tree／tab completion、statistics／advancements、client settings／brand／payload、resource pack／NBT query | [旧版packet coverage](protocol-coverage.md)はnativeの全92分岐を示すだけ。共通型・能力宣言・bounded受信とrequest ID所有を追加。権限による拒否も実接続で検査。resource packは要求/statusのprotocol面が先で、asset download／描画は別 |
| LIFECYCLE / P2–P3 | 広いscene／編集／replay／reconfiguration／chunk再観測・明示復旧、遮断後の全履歴可読性、online認証・暗号化・署名chat、追加版 | raw recordingと選択replayは既存。操作権限を復元せず、切断／取消／buffer上限／旧IDを検査。onlineと追加版は別のP3導入機能として分離し、offline対応をonline対応としない |

広いsceneには、native modern拡張のmoving-piston carrier／回路再構成、slime／honey付着、
block callback、仮想編集・予測の連鎖・assumed scene・光の再構成も含む。
nativeで検証済みでも、共通の両版契約へ接続するまでは共通対応済みに加算しない。
受信光とローカルに計算する光は区別する。[過去の回路rollout](client-rollout-roadmap.md)と
[共通scene](common-recording-scenes.md)を参照する。

Issueへのリンクは下の追跡欄に記録する。既存[PR #35](https://github.com/sugipamo/Voxrig/pull/35)の
history読取時計／cursor ordinalはOBS-ENTITYの未実装分へ重複登録しない。
[Issue #5](https://github.com/sugipamo/Voxrig/issues/5)の10-client timeout調査は利用者の指示で保留し、
この作業の完了条件へ含めない。原因が解決したとも扱わない。

## 検証の範囲

| 確認済みの入口・場面 | 再実行と証拠 |
| --- | --- |
| 代表的な収納・製作・採掘復旧・UI・manager・乗車／下車 | [common-client-native-validation](common-client-native-validation.md)、`examples/common_native_probe.rs` |
| 水中・梯子・つる・足場の継続操作 | `scripts/run_climbing_control.py`、[common-control](common-control.md) |
| session所有の単発接地ジャンプ・押下入力保持・取消境界 | `scripts/run_ground_jump.py`、[公式両版21checkの証拠](evidence/common-ground-jump-20261009.json) |
| player泡の柱、ボートの水源／水没／流水・解除・強制下車 | `scripts/run_fluid_control.py`、[公式34checkの証拠](evidence/common-fluid-control-20261009.json)、[3346tickのoracle](movement-oracle.md) |
| ボートの泡・速度受信元・待機取消・強制下車 | `scripts/run_boat_bubbles.py`、[公式両版48checkの証拠](evidence/common-boat-bubbles-20261009.json)、32実行・1,080tickの元の移動照合 |
| ボートのスライム・ベッド・クモの巣・蜂蜜 | `scripts/run_boat_hooks.py`、公式両版40check、28実行・1,470tickの元の非LivingEntity callback照合。他entityとの衝突は継続対象 |
| 64block ray、mode別look／hotbar | `scripts/run_migration_api.py`、[両版42checkの証拠](evidence/common-migration-api-20261009.json) |
| entity寿命／受信履歴 | `scripts/run_entity_history.py`、[両版8checkの証拠](evidence/common-entity-history-20261009.json) |
| 履歴読取時計・空page・world変更・遮断後 | [今回の公式両版14checkの証拠](evidence/common-consumer-history-clock-20261009.json)。既存runnerと追加clock検査の実行ソース・hashを保持 |
| entity health／equipment／named dataと使用中状態 | `examples/entity_block_probe.rs`、`entity_data_probe.rs`、`item_use_probe.rs`。各common文書の検証節 |
| native元イベント容量（1.16.1） | `scripts/run_native_event_capacity.py`、[common-events](common-events.md)。modern明示指定は接続前拒否 |

この文書と能力一覧exampleの追加で新しいゲーム内機能は増えない。過去の検証を今回の新規実接続として数えない。
今回の追加実接続はPR #35の履歴時計を含む同じSDKソースを対象とし、両版各7checkが成功した。
既存sampleの時刻が変わらずageが増えること、空pageの時計、world変更・revocation後の同じ時計を確認した。
検証サーバーは終了済み。証拠JSONのSDK commitとsource treeは実行時のものを記録している。
追加機能はそれぞれ公式両版で同じcommon consumerを通し、
成功だけでなく欠測・拒否・取消・切断・world／ID／screen再利用も記録する。
原packet、SDKソースcommit／tree、公式JAR hash、結果との対応を保存する。
非公開利用側のworkflowはそのプロジェクトで固定commitを使って確認する。

## 利用側に置く機能

目的地選択、pathfinding、長期記憶、worldの意味認識、道具／食事／材料／建築計画、
combat戦術、役割分担、自動再接続方針は利用側が持つ。
Voxrigは、その判断に必要な受信事実、指定された低水準操作、結果の同期・不確実性を提供する。

## 追跡

[親Issue #39](https://github.com/sugipamo/Voxrig/issues/39)から全体を追跡する。

| 群 | Issue |
| --- | --- |
| OBS-ENTITY | [#40](https://github.com/sugipamo/Voxrig/issues/40) |
| OBS-WORLD | [#41](https://github.com/sugipamo/Voxrig/issues/41) |
| INVENTORY | [#42](https://github.com/sugipamo/Voxrig/issues/42) |
| WINDOWS | [#43](https://github.com/sugipamo/Voxrig/issues/43) |
| MOVEMENT | [#44](https://github.com/sugipamo/Voxrig/issues/44) |
| INTERACTION | [#45](https://github.com/sugipamo/Voxrig/issues/45) |
| PROTOCOL | [#46](https://github.com/sugipamo/Voxrig/issues/46) |
| LIFECYCLE | [#47](https://github.com/sugipamo/Voxrig/issues/47) |

INVENTORYのarmor／offhand交換は既存#38で追跡し、#42はそれ以外の残件。
MOVEMENTの拡張#44とは別に、再開不具合#37はPR #50で修正済み。単発接地ジャンプは#49で追跡する。
