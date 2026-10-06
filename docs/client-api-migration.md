# 0.2 client APIへの移行

registry entryの共通検索は`server_registry_state().await?`から
`find_entry(registry, name)`/`bind_entry(registry, native_id)`で行い、`entry_name(&id)`で解決する。
`RegistryEntryId`が版・registry名・固定/実受信の所有範囲を保持するので、数値だけを保存しない。
legacyのenchantmentは固定、modernはserver設定に属するが、検索側のversion分岐は不要である。
固定値だけを検索する場合は`client.registry().builtin_id(...)`を使う。
legacyのdimension codecは個別entryも取得できる。新旧とも未受信の動的entryは利用不能のまま扱う。

利用側のソース公開は不要です。各環境で以下の移行を行い、main上の固定commitまたはreleaseを基準に検証します。

`target_block`はsurvival/creative両handleで同じ`BlockTargetObservation`を返します。
選んだhandleと受信modeの一致を検査し、active flightは現在未対応です。
共通型は`client::{BlockTargetHit, BlockTargetObservation}`からもimportでき、従来のsurvival importは同じ型です。
`Feature::BlockTargeting`は共通queryの対応情報、従来の`SurvivalTargeting`も維持します。
両版のchest等7種類・全102storage stateを狙えるようになりましたが、採掘/設置の許可や
共通container openの結果へ読み替えないでください。詳細は[狙い判定](common-survival-targeting.md)を参照。

## deepplanning派生版から

共通の基本entity操作には、`Client::entity_spawns()`で受信した`EntityId`を渡す。
`survival()`／`creative()`の`interact_entity`・`attack_entity`は同じ引数で使え、受信modeと元のspawn寿命を検査する。
従来の整数entity IDを直接渡すコードは、共通captureから対象を選ぶ形へ変更する。
`spawn_position`は受信spawnの履歴なので、従来の現在位置・metadata付きentity queryの代替とは扱わない。
それらの広い共通観測は残る対応範囲に含む。

従来の`Bot::attack`に含まれたcooldown待ち・arm swingは、共通`attack_entity`では実行しない。
一回のATTACK送信を提供するため、攻撃間隔・武器・対象・結果の判断は利用側へ移す。
基本装備は既存の`transfer_inventory(InventorySource::Player, slot)`を利用し、
完了判定には`InventoryTransferStage::ObservedTransferred`と実変更slotを使う。
詳細は[基本装備とentity操作](common-client-entities.md)を参照。

依存packageを`zen-minecraft-client`から`voxrig`へ変更し、Rustのimportを`voxrig`へ変更します。
1.16.1の詳細APIは`voxrig::versions::java_1_16_1`でも参照できます。

| 旧名 | 公開client API |
| --- | --- |
| `ClientConnectionGeneration` | `ConnectionGeneration` |
| `ClientOperationClass` | `OperationClass` |
| `ConnectionLifecycle` | `ConnectionState` |
| `PrimitiveOperation` | `Operation` |
| `AcknowledgedPrimitive` | `AcknowledgedOperation` |
| `CleanupPrimitive` | `CleanupOperation` |
| `PrimitiveDiagnosticCorrelationId` | `DiagnosticCorrelationId` |
| `PrimitiveDispatchOutcome` | `DispatchOutcome` |
| `PrimitiveDispatchError` | `DispatchError` |
| `PrimitiveRequest` | `OperationRequest` |
| `CraftClick` | `WindowClick` |
| `CraftExecution` | `WindowClickSequence` |
| `CraftAcceptedCacheEffects` | `WindowPrediction` |
| `CraftAcceptedSlotEffect` | `SlotPrediction` |
| `SensorCaptureIdentity` | `CaptureIdentity` |
| `LoadedResourceQuerySnapshot` | `BlockQuerySnapshot` |
| `LoadedResourceCoverage` | `BlockQueryCoverage` |
| `LoadedResourceQuery` | `BlockQuery` |
| `TraversalMovementFactsRequest` | `MovementSnapshotRequest` |
| `TraversalMovementFactsSnapshot` | `MovementSnapshot` |
| `TraversalGeometryQuery` | `GeometryQuery` |
| `TraversalGeometrySnapshot` | `GeometrySnapshot` |
| `TraversalGeometryBlockFact` | `GeometryBlock` |
| `TraversalBlockFact` | `MovementBlock` |
| `TraversalEntityDimensions` | `EntityDimensions` |
| `TraversalEntityFact` | `ObservedEntity` |
| `TraversalInventorySlotFact` | `InventorySlotFact` |
| `TraversalInventoryFact` | `InventoryFact` |
| `body_interest_generation` | `interest_generation` |
| `body_generation` | `generation` |
| `observation_interest` | `interest` |
| `capture_traversal_movement_facts` | `capture_movement_snapshot` |
| `query_traversal_geometry` | `query_geometry` |
| `query_loaded_resource` | `query_loaded_blocks` |
| `dispatch_primitive_with_diagnostic` | `dispatch_operation_with_diagnostic` |
| `dispatch_primitive` | `dispatch_operation` |
| `dispatch_craft` | `dispatch_window_clicks` |
| `ZEN_R9_INSTRUMENTATION` | `VOXRIG_TRACE_OPERATIONS` |
| `zen_client_dig_lifecycle` | `voxrig_dig_lifecycle` |
| `zen_minecraft_client` | `voxrig` |
| `zen-minecraft-client` | `voxrig` |
| `connection_lifecycle` | `connection_state` |
| `accepted_cache_effects` | `prediction` |
| `primitive module` | `versions::java_1_16_1::operation` |
| `connection lifecycle module` | `versions::java_1_16_1::lifecycle` |

`FurnaceExecution`と`FurnaceExecutionPhase`、`dispatch_furnace`は削除しました。
かまど操作は`WindowClickSequence { window_id, close_window_after: false, clicks }`を
`dispatch_window_clicks(context, sequence)`へ渡します。input/fuel/outputをどこへ置くかは利用側の計画です。
旧phaseはclientのdispatchで使われていませんでした。利用側がphaseを追跡する必要がある場合は利用側に保持します。
共通Clientでは`furnace_state()`／`FurnaceSlot`と、mode handleの`open_container`・
`click_inventory`・`close_container`を使用します。native window/menu ID・player offsetを利用側で分岐せず、
精錬結果は実output受信で確認します。詳しい条件は[共通かまど操作](common-furnaces.md)を参照。

`WindowPrediction`はserver受信値ではありません。既存のcache予測の契約を維持しますが、
ackやcache更新だけを目的達成の証拠にしないでください。

## minetoolから

`voxrig`のpackage名、`Bot`/`BotManager`、通常のモジュールimportは維持します。
追加fieldを持つ`ControlState`や`InventoryState`は`..Default::default()`で構築するか、追加fieldを明示します。
直接構築する`SlotUpdate`/`WindowTransaction`/`ItemCollected`にはpacket sequence等の追加fieldが必要です。
Window Itemsで確定したplayer slot offsetを保持し、部分更新でinventされたoffsetを使いません。

## DustRouteから

`Client`、`ConnectionConfig`、`MinecraftVersion`、既存の1.21.11操作メソッド名は維持します。
vendorの内容をmain上の固定commitで置き換えるか、同じcommitをGit dependencyの`rev`へ指定します。
Cargo.lockも更新し、`voxrig` feature付きのbridge・operation・recording試験を行います。

1.21.11の`Inventory`はscreen revision・cursor・pending swapを含むようになり、
内部のslot sequenceをprivate fieldとして保持します。利用側のfixtureで直接struct literalを
構築していた場合は`Inventory::default()`を作り、必要な公開fieldを後から設定してください。
通常は操作APIから受信済みsnapshotを取得します。

サバイバルの在庫交換は`swap_player_hotbar`へ送信し、その送信記録を
`wait_inventory_swap`へ渡します。timeoutや取消の後も送信を繰り返さず、
同じ接続の読出し待機を再開して両slotの新しい受信を確認します。

1.21.11の`PlayerState`に`local_player: LocalPlayerState`が追加されます。
直接struct literalを構築するfixtureはこのfieldも指定してください。
通常は`player_state()`でnative初期値・受信sequence付きの観測を取得します。
`standing_context()`は静止した通常立位と対応geometryに限定され、サバイバルの
`look()`も送信前にこの条件を確認します。未ロード、姿勢・速度の欠測、液体等の未対応
geometryではエラーを利用側へ返し、送信成功や通常歩行の保証へ読み替えないでください。
クリエイティブの視点変更は既存の経路を維持します。

1.21.11の追加統合では`PlayerState`に`interaction_loading`、`motion`、`selected_hotbar`、
`ObservedPlayer`に`motion`、`StandingContext`に`position_basis`も追加されています。
直接構築するfixtureはこれらを指定してください。`LocalPlayerState`の移動属性の追加は
`..Default::default()`を使うfixtureでは省略できますが、操作の許可には実際の基準観測が必要です。
`position_from_server`は`OwnMotion`から導出する互換fieldであり、予測位置を受信値へ変えません。

1.21.11の`wait_until_ready()`はworldごとのloading通知完了も待ちます。
位置だけをseedしたTCP fixtureでmutationを試す場合は、実際のINITIAL_CHUNKS_COMING・
chunk・位置受信の経路も用意してください。取消・write失敗の後は`UncertainDispatch`を扱い、
閉じた接続から`operation_history()`で診断情報を読んでください。履歴から操作を再送しません。

survivalのraw `use_on_block`は拒否します。`place_survival_cube`で意図を登録し、
対象block・材料消費・処理sequenceを読出し待機で確認してください。
採掘ではair受信後も元接続のmutationは保留され、明示的なfresh recoveryを行います。
独立観測方式では別接続の新しいUUID削除受信が必要です。単一プロフィール方式は以下を参照。
既存の移動入口は有限の`SurvivalInput`列と別接続のobserverを渡し、予測・観測・誤差を区別します。
壁際停止の失敗は`RequiresInspection`として保持され、自動再開しません。
`SurvivalMovementPreview`に`terminal_clearance`、`SurvivalMotionRecord`に`recheck`も追加されます。
`TerminalClearance::RequiresReplan`の入力列は送信できないため、退避を含めて利用側で計画し直します。
完全送信・静止予測を持つ失敗runの明示的な再観測は`prepare_survival_motion_recheck` /
`observe_survival_motion_recheck`へ分離されます。中断や補正を観測だけで取り消すAPIではありません。
詳細は[採掘](survival-mining.md)、[retirement](survival-mining-retirement.md)、
[配置](survival-placement.md)、[移動制御](survival-motion-controls.md)を参照してください。

## 1.16.1の製作・収納の追加修正

`Bot::compact_player_inventory(window_id)`はcursorを通常収納へ戻し、同じitem・NBTの
stackを上限までまとめます。player windowでは収納可能なoffhandも通常収納へ戻すため、
装備を維持したい利用側はこの操作による変化を考慮してください。装備slotを退避先には使いません。
`craft_once`は製作前にこの整理を実行し、残ったgrid材料を戻してから新しいrecipeを置きます。
製作結果は取得前に収納容量を確認し、空きslotだけでなく互換stackの残容量も使います。
拒否・不一致では停止します。一部クリック後の失敗はrollbackや再試行許可ではなく、
新しい観測で残ったcursor・grid・収納を確認してください。protocol確認後のcache予測を含む
1.16.1の在庫契約は維持し、独立したゲーム内成功の証拠とは扱いません。

## 検査付きサバイバル入口と追加の互換性変更

版固有の1.21.11操作APIは共存します。対応するサバイバル操作は
`Client::checked_survival()`から取得し、型を`voxrig::client::survival::checked`からimportできます。
`survival_capabilities()`は静的な対応情報で、実行時の状態・権限確認は各呼出しに残ります。
1.16.1にはこの契約を適用できず、従来の`Bot`と`survival`モジュールを使います。

| 型・入口 | 今回の変更と利用側の対応 |
| --- | --- |
| `SurvivalMovementPreview` | `yaw`を削除し、`generation`と`controls: Vec<SurvivalControl>`へ変更。headingは各controlの`yaw`から参照する |
| `SurvivalMotionRecord` | `inputs`を削除。`preview.controls`の各tickの`input`と`yaw`を参照する |
| 固定headingの`preview_survival_motion` / `start_survival_motion` | 引数は維持。可変headingは`preview_survival_path` / `start_survival_path`を使う |
| `start_previewed_survival_motion` | previewを現在のstateから再計算して照合する。古いpreviewやhypothetical previewは実行許可にならない |
| `MiningRecord` | `inventory_change: Option<MiningInventoryChange>`を追加。直接構築するfixtureとserialized historyの利用側を更新する |
| `dig_survival_cube` | FINISH前に受信した前提変更をtyped `RequiresInspection`として返す。transport failureは引き続きerrorで、再送しない |
| `MiningRetirement` | 元接続・observer・watchをまとめた非永続handle。明示的なclose、read-only wait、once-only reconnectに分ける |
| `RecoveredSurvivalClient` | 同じ新sessionの汎用`client`、検査付き`operations`、診断`evidence`を返す。利用側で新しい計画を作る |

仮想場面は`capture_survival_scene`から枝を作り、共有native modelで予測します。
仮想block編集は受信・在庫・権限・結果ではありません。`HypotheticalAimRequirement`を
将来の観測条件として保持し、実行時の立位・対象・材料・結果観測へ置き換えないでください。
詳細は[API契約](survival-api.md)、[仮想場面](survival-hypothetical-scenes.md)、
[受信中の採掘前提変更](survival-inventory-interruption.md)を参照してください。

### 視線の誤差判定と仮想再接続の追加

1.21.11の採掘・配置は、位置誤差を含む視線の連続した通過範囲を検査します。
視線から外れた足元の支持blockによる過剰な拒否を解消しますが、実際の遮蔽物・未知cell・
面の端・reachの曖昧さは拒否します。立位や操作結果の確認条件は引き続き必要です。

`SurvivalMovementPreview`と`HypotheticalMovementPreview`にtick 0の`initial_frame`を追加します。
前者を直接構築するfixture、previewを保存・表示するschemaを更新してください。
同じ座標でも移動後の静止と新接続の初期状態では速度・接地flagが異なるため、
開始位置だけで予測の同一性を判定しないでください。

`HypotheticalAimRequirement::ReceivedAfterReconnect`を追加します。網羅的な`match`を更新してください。
`SurvivalScenario::after_expected_reconnect()`は安全な停止点から新接続を仮定した枝と
`HypotheticalReconnectBoundary`を返します。実際の再接続後に取得したsceneを
`validate_received_start(&fresh, retired_connection_id)`で照合します。
同じ接続・異なるdimensionや足位置・受信由来でない立位を拒否する比較であり、
retirementの証明、world内容とcaptureの新しさは利用側が別途確認します。
この呼出しは切断・再接続・送信を行わず、実操作の現在の検査を置き換えません。

## 単一クライアントの復旧・予測契約と1.16.1追加イベント

| 型・入口 | 変更と利用側の対応 |
| --- | --- |
| `SurvivalCapabilities` | `same_profile_mining_recovery`、`prediction_based_contract`を追加。struct literalと保存schemaを更新。1.16.1ではfalse / None |
| `SurvivalContract` | `PredictedDryCubeV1`を追加。既存の`checked_contract`は`ObservedDryCubeV1`。網羅的なmatchを更新 |
| `SurvivalMotionRecord` | `contract`を追加。`observer_connection_id`と`initial_watch`はOptionへ変更。予測契約ではNoneで、観測を補完しない |
| `SurvivalMotionStatus` / `StandingPositionBasis` | `Predicted`を追加。実測やserverの停止確認として扱わない |
| `MiningRecord` | `recovery_attempt: Option<MiningRecoveryAttempt>`を追加。取消・失敗後も履歴に残る |
| `MiningRecoveryEvidence` | `retirement`を削除し、`boundary: MiningRecoveryBoundary`へ変更。独立方式は`IndependentRemoval { receipt }`、同一プロフィールは`SameProfileLogin { uuid, name }`として処理する |
| `MiningProfileRecovery` | `prepare_mining_profile_recovery`で準備し、明示的なcloseと一度だけのreconnectを行う。独立方式と再接続guardを共有 |
| `MiningRecoveryTarget` | 同一プロフィール方式のreconnectへ`Exact(original_or_air_state)`または`OriginalOrAir`を渡す。対象の照合条件であり編集・再採掘の許可ではない |
| `HypotheticalAimRequirement` | `PredictedEndpoint { planning_reserve }`を追加。独立観測条件を予測だけで満たさない。`validate_standing`の比較は操作許可ではない |
| `HypotheticalMovementPreview` | `endpoint_contract`を追加。仮想計画では`scenario_with_motion_contract`で将来の根拠を選択する |
| 1.16.1 `Event` | `InventorySlotObserved(Snapshot<SlotUpdate>)`を追加。網羅的なmatchを更新。既存`SlotUpdated`との二重通知を二重操作へ変換しない |

observerなしの移動は`start_predicted_survival_path`または
`start_previewed_predicted_survival_motion`で明示します。水平1/16blockのreserveは
model内の方針であり物理誤差の保証ではありません。補正・中断・geometry変更時の拒否と、
次の操作ごとの新しい検査・配置結果の受信確認は必要です。
同一プロフィール復旧は直接接続した未改造vanilla 1.21.11とプロフィールの排他的所有が前提です。
両復旧方式ともlogin I/O前にclaimを保持し、取消・失敗後のcloneや方式変更でも再試行しません。
元接続は閉じたままで、利用側の計画や永続jobを引き継ぎません。
[予測契約](survival-predicted-motion.md)と[同一プロフィール復旧](survival-single-profile-recovery.md)を参照してください。

1.16.1の新イベントは適用時点のrevisionとSet Slot受信内容を保持します。
同一接続の現在のinventory snapshotとの比較に使い、queueから読んだ時点のrevisionを
受信時点のものとして補完しないでください。クリック拒否のerror文字列にはwindow・slot・
button・modeが追加されます。文字列に依存する診断は更新してください。
液体の壁脱出・浅い液体でのjumpを使う採用先は実サーバーでも確認してください。

## 採用検証

Voxrig commit、利用側commit、ゲーム版、Rust toolchain、実行コマンドと結果を記録します。
deepplanning: coherent observation、context付き操作、製作・かまど・採取、取消・切断。
minetool: イベント過多、閉じたwindowの遅延更新、作物掘削、原木から石ツルハシ製作。
DustRoute: native observation、piston/recovery、照準、配置・除去・取消・recording。
各環境の結果は使用した固定Voxrig commitと共に記録します。現在はmainを常設し、
採用検証の未実施項目も別途保持します。mainへの統合を各利用環境での成功の証明にはしません。

## Client共通化ブランチの移行

専用ブランチ`codex/client-api-unification`の変更。全段階の統合はまだ完了していません。
詳細と残作業は[Client共通化](client-unification.md)を参照してください。

| 従来の入口 | 移行先 |
| --- | --- |
| 新しいconsumerの`voxrig::prelude::*` | `voxrig::client::prelude::*`。root preludeは従来Bot用のまま |
| `voxrig::client::{Bot, Event, Player, ConnectionOptions}` | 版固有APIを使い続ける場合は`voxrig::versions::java_1_16_1::client`またはrootからimport |
| `ConnectionConfig.limits: ConnectionOptions` | `ClientLimits`。共通の4 timeoutとmax_chunksのみ。版固有設定はBot APIに残る |
| `client.survival()?`の検査付き操作 | `client.survival().checked()?`または`client.checked_survival()?`。既存の制約・証拠は維持 |
| `voxrig::checked_survival` | canonicalは`voxrig::client::survival::checked`。旧pathはaliasとして維持 |
| modern/checkedの`SurvivalInput` / `SurvivalControl` / `PredictedMotionFrame` / `TerminalClearance` | canonicalは`voxrig::client::survival`。従来importも同じ型をre-exportする |
| finite `start_predicted_survival_path` / `survival_motion` | 共通は`client.survival().start_predicted_path` / `motion_record`。`MotionRecord`を保持し、prediction契約・取消後のowner・before-I/O intentを両版で維持 |
| read-only `preview_survival_path` | 共通では`client.survival().preview_path`。両版で`MotionPreview`を返す。追加checked契約の戻り値とは区別 |
| default cubeの`place_survival_cube` / legacyのraw block use | 共通は`client.survival().place_cube(support, face)`。`PlacementRecord`で対象・1個の材料消費・版固有processingを分けて保持 |
| player main/hotbarの版専用SWAP | `client.survival().swap_hotbar(main, hotbar)`または`client.creative().swap_hotbar(...)`。`InventorySwapRecord`で実受信・legacy応答・未解決状態を保持 |
| `client.java_1_21_11_operations()`でのcreative基本操作 | `client.creative()`。`set_creative_hotbar`→`set_hotbar`、`dig_creative`→`break_block`。戻り値はDispatchReceipt |
| PlayerStateを共通playerとして使用 | `client.player_state()`のPlayerObservation。追加検査契約のnative PlayerStateとは区別 |
| 版なしの`item_id`/`item_name`など | `client.registry()`。整数IDはversion/kind付きRegistryIdとして保持 |
| vanilla fixtureの動的registry IDを接続先へ流用 | `client.server_registry_state().await?`。受信済みentryから`find`/`bind`し、connection/configuration付きServerRegistryIdを保持。legacy dimension codecの個別entryも元順序・元bytesで解決 |
| itemとregistryを別々にcaptureし、後から組み合わせる | `client.received_inventory().await?`。読み取り専用のslot/itemが同時取得したregistryと実受信ordinalを保持。予測cacheを含めず、古いitemへ新しいconfigurationを付け替えない |
| itemの元NBT/patch bytesやCRCをnative stackの同一性に使う | 同一configurationの`ReceivedItem::native_equivalent(&other)`。legacy constructor、modern prototypeとpatch、実受信registry参照を適用。未解決text内item/dialog等はエラーを保持し、操作許可に使わない |
| metadataをNBT/patchの元bytesから独自に取得 | 両版共通の`item.custom_data()?`。typed `NbtData`を読み、受信の根拠は外側の`ObservedValue.source`を参照。全itemの意味・inventory hashは別契約 |
| static item定義の容量・独自NBT/patch解析を現在itemのpropertyとして使う | `item.properties()?`。default/追加/削除/版別の受信補正を使う。同じfield名で容量・耐久・stackableを読む。signed native値を保ち、slot規則や操作許可として使わない |
| 任意のshort item name | 共通APIでは`minecraft:stone`等のnamespaceを明示 |

Client共通入口ではoffline名は3～16のASCII英数字/underscore。不正名・空host/port 0、
ゼロtimeout/chunk上限は接続前に拒否する。既存直接Bot importの通常動作は維持する。
Survival/Creative handle取得はResultを返さず、実際のmode・permissionはmutationの直前に検査する。
「handleがあるからそのmodeになった」という判定へ移行しない。

common inventoryの受信slotを従来のcache予測で補完しない。NoneをEmptyに変換しない。
`DispatchReceipt`を成功した配置・移動・採掘の証拠として保存しない。

MinecraftVersionと拡張予定の共通enumはnon_exhaustiveです。利用側でmatchする場合はwildcardを設け、
通常操作にversion分岐を置かない構成へ移行してください。新規adapter追加で通常のconsumerコードを変更しないための境界です。

## 共通Survivalの狙い判定

版専用のown-player raycastを呼ぶ利用側は、限定dry standingであれば
`client.survival().target_block(4.5)`から共通の`BlockTargetObservation`を取得できる。
`initial`の観測根拠と`hit`のmodel計算を分けて扱う。queryは採掘/設置許可やserver受理の証拠ではない。
1.16.1は現在の12素材のoutline、1.21.11は既存のstatic outlineが対象。
任意のentity/fluid/shapeを既知のcubeへ置き換えない。詳細は[共通狙い判定](common-survival-targeting.md)を参照。

## 共通Clientの採掘へ移行

通常の利用側は`client.survival().start_mining(target, face)`と返された`MiningId`を使用し、
`finish_mining(id)` / `abort_mining(id)`を明示的に呼ぶ。診断は`mining_record()`から取得する。
legacyの時間指定`dig_block`をそのままtimeout/retryのループへ移植せず、I/O前に保持される
共通attemptを扱う。modernのnative `MiningIntent`と共通`MiningId`は別の公開契約で混ぜない。
`MiningInventoryChangeKind`は共通側が所有し、modernの旧importは同じ型のre-exportとなる。

Clientの`pending_dispatch`は在庫/位置に加え、保持中の採掘、未解決の設置/有限移動も含む。
common preview/target queryは未解決dispatchがあれば拒否する。creative writeに対する
RCON確認をClientのslot受信として扱わず、実際のinventory updateを待つ。
採掘airやABORTを次の操作許可とせず、共通fresh recoveryは後続段階として扱う。
前提・stage・取消の扱いは[共通Survivalの採掘](common-survival-mining.md)を参照。

## 共通Clientの設置へ移行

通常のdefault cube設置は`place_cube(support, face)`を一度だけ呼び、結果は`placement_record()`で読む。
`PlacementId`は診断用のopaqueなattempt識別子で、modernのnative `PlacementIntent`と混ぜない。
送信済みでも対象と材料の実受信が揃うまで`Pending`。legacyの処理ACKは存在せず`None`、
modernは送信より新しい実ACKのordinalも要求する。timeoutや取消で再送しない。
完了後の次操作は新しい場所・材料・身体の条件を再検証する。

1.16.1のSet Slot window -2はraw Inventory番号を使うため、hotbar 0..8をplayer screen 36..44へ変換する。
受信在庫と従来cacheの両方を修正し、特殊window -2を開いたcontainerやcrafting slotとして扱わない。
cursorやactive windowを更新したことにもならない。詳細は[共通設置](common-survival-placement.md)を参照。

## 共通Clientの在庫交換へ移行

main screen slot 9..35とhotbar index 0..8の通常交換は、modeに合うhandleの`swap_hotbar`を一度だけ呼ぶ。
結果はどちらのhandleからも`inventory_swap_record()`で読み出せる。`Pending`でも再送しない。
共通`InventorySwapId`とmodern専用`InventorySwap`は別契約で、nativeのwait APIへ共通recordを渡さない。
実受信empty cursor/両slot・player screenを要求し、legacyではmatching比較応答も照合する。
解決できるlegacy NBT/modern component付きstackにも同じAPIを使い、実効容量と受信registryを検査する。
結果の成功判定は元bytesの一致ではなく、fresh receiptのnative field比較を使う。実クリック後のnegative比較応答はnative resyncを表し、rollbackと扱わない。
modernの画面revisionとlegacyのtransaction番号を共通の成功ACKとして扱わない。
完了後の次操作は新しいbaselineから開始する。crafting・data付きQUICK_MOVE/cursor付きcloseはこの操作だけで対応済みにはならない。
前提・取消・履歴と検証は[共通在庫交換](common-inventory-swaps.md)を参照。

storage交換はnative slotの受入れ条件も送信前に検査する。shulker boxへのshulker box収納は
`InvalidInput`で拒否し、clickや未解決recordを作らない。通常PICKUPは`click_inventory`へ共通化する。shift-clickは`transfer_inventory`を使用する。
1.16.1の`warped_fungus_on_a_stick`最大容量はnativeに合わせ1へ修正した。
版別の容量は`client.registry().item(...)`から読み、upstream値64や共通の固定容量を仮定しない。
[nativeクリック調査](common-inventory-clicks.md)に元の実装と再生成手順を記載する。

## 共通のcontainer観測へ移行

開いた画面は`Client::screen_state()`で取得する。`screen.slots`とplayer screen 0..45の在庫を混ぜない。
playerとの対応は`screen.layout.player_slots`を使い、末尾36slotや件数だけから推定しない。
layout未確定なら対応は未確定のまま扱う。`ScreenId`全体でopeningを比較し、window IDだけを再利用しない。
modern側は外国windowのfull内容を保持するようになった。malformed packetを途中で適用しないため、
cursorを省いた不完全なfixture（例`[1,0,0]`）はエラーになる。default empty cursorまで含む実wireへ修正する。
legacyのfull内容からcursorをEmptyと推定せず、実cursor updateを待つ。
詳細とAPI範囲は[コンテナ画面観測](common-container-observation.md)を参照。

## 共通のstorage/hotbar交換とrecord名

既に開いたstorageのwhole stack交換は、modeに合うhandleの`swap_container_hotbar(screen.id, slot, hotbar)`を一度だけ呼ぶ。
結果はplayer交換と同じ`InventorySwapRecord`で読み出す。元openingが変わったら旧recordで再送しない。

| 共通recordの旧field | 新field/意味 |
| --- | --- |
| `main_slot: u8` | `source_slot: u16`。実クリック画面内のsource slot |
| `main_before` | `source_before`。完全な実受信predecessor |
| `main_receipt` | `source_receipt`。反対側から移ったstackのfresh receipt |
| 新規 | `source: InventorySwapSource`でPlayerMain/Container openingを区別 |
| 新規 | `initial_screen`は同じboundaryの実container baseline。player交換はNone |
| 新規 | `hotbar_screen_slot`は実native menuのhotbar slot。`hotbar`は従来通り0..8 |

共通recordを参照/JSON集計する利用側はこの名前へ移行する。
modern専用のnative `InventorySwap`/`InventorySwapObservation`の`main_*` fieldは変更していない。
共通recordとnative専用submissionは別契約で、native waitでcommon ownerを解除しない。

## container closeの共通入口

legacyの`Bot::close_window()`等に代わる共通入口は、modeに合うhandleの`close_container(screen.id)`。
`ContainerCloseRecord.dispatched`を完全送信の確認に使い、`ObservedClosed`を必ず待つ設計にはしない。
Vanillaは通常closeをechoしないため、`server_close_sequence == None`でも完全送信は記録できる。
`Client::screen_state()`は実受信履歴なのでclose送信だけでは元画面を消さない。
元画面へclick/closeを再送せず、`container_close_record()`で送信不確実性を確認する。
旧cacheのscreen消去をサーバー側閉鎖確認へ読み替えない。
新しいOPENを実受信すればその新identityから次操作を明示的に始める。
実cursor返却、取消後のowned処理、player screen再開/open契約は
[共通container close](common-container-close.md)を参照。

## close後のplayer画面とactual player revision

共通`InventoryObservation`には`player_screen`と`player_screen_revision`を追加する。
共通`ScreenObservation`には同じ`player_screen`を追加する。
これらをstruct literalで作るfixtureは新fieldを用意し、受信/dispatchの根拠を持たない値をReceivedとしない。
serialized recordの解析側は追加fieldを扱う。

UIがplayerへ戻ったことを`window_id == Some(0)`だけで判定していた利用側は、`player_screen`のbasisを確認する。
`SubmittedClose { close }`はそのClientが完全にcloseを送った根拠であり、serverの実close応答とは別である。
通常在庫の共通入口`swap_hotbar` / `inventory_swap_record`の名前・引数は変えず、close後にも実装する。
modernではclose後に別のmenu revisionや0を注入せず、actual player-screen-zero revisionを別に保持する。
欠測の場合は再送せず、新しいreceived contextを検討する。
既存modern native-only inventory APIのreceived-window制約は維持する。
詳細は[共通プレイヤー画面](common-player-screen.md)を参照。

## Storage activationの共通化

両modeで`open_container([x,y,z])`と`container_open_record()`を使う。
empty main hand/offhand/cursor、実player UI、default dry standing、監査済みstorageのfirst outlineを要求する。
creative `use_on_block`の固定cursorによるcontainer opening fixtureから移行できる。
survival activationも独立実装し、modernのraw survival use-on-block拒否を解除していない。
送信済みを開いたとせず、actual OPEN/full/cursorとmodern actual processing ACKを別に保持する。
画面を開かなかった場合も同じintentを再送しない。OPENにはtarget座標がない。
[契約と対応範囲](common-container-open.md)を確認して、受信したscreen IDで次のswap/closeを要求する。
legacy native `OperationAdmissionError`に`BoundedContainerOpenInProgress`を追加したため、
そのenumのexhaustive matchは移行が必要。共通Client consumerはこのnative enumに依存する必要はない。

`ContainerOpenRecord.target`は初期state、`target_state`は最後のworld cache inspectionを表す。
後者の`capture_sequence`はblock固有の更新ordinalではない。通常barrelのopen boolean更新は
outline/menuを変えず、完全送信後だけ互換として扱う。他propertyや送信前の変化は許可しない。

## 通常クリック・split・返却

両mode handleで`click_inventory(source, slot, InventoryClickButton::{Left,Right})`を使用する。
プレイヤー在庫は`InventoryClickSource::Player`とinput slot 1..4 / inventory slot 9..44、storage・crafting tableは受信した`ScreenId`を
`InventoryClickSource::Container { screen }`へ渡す。storage側は付属player slotも指定できる。
SWAPのhotbar引数とは異なり、ここはnative screen slot番号を渡す。

返されたrecordはI/O前のintentと別の予測を保持する。`inventory_click_record()`を読み、
`ObservedClicked`で両方のfresh実source/cursorを確認してから次へ進む。timeout/取消/競合時に
同じclickを繰り返さない。`RequiresInspection`は後から値が復元されても保持する。
両handleのgetterは同じ接続の最新clickを読むので、modeが変わっても履歴を調査できる。
旧native `OperationAdmissionError`には`BoundedInventoryClickInProgress`を追加するため、
このenumを網羅matchしている利用側は対応するarmを追加する。

通常PICKUPの既知itemは、受信legacy NBT/modern components付きでも、監査済みstorageとplayer main/hotbarで使用できる。
実効容量と元receiptのregistryを使い、予測を実受信へ昇格しない。modern data付き操作の
`send.sent_screen_revision`は再同期用であり、実受信`screen_revision`と区別する。
Leftはlegacy default constructorのNBTを保持して移動し、
modern default bundleも空きcursor/空きslotとの移動に対応する。bundle内部の収納やRight override、
未解決data・特殊item override、PICKUPのresult/armor/offhandは追加対応を要する。cursor付きcloseは下記の共通返却へ移行する。
詳細は[通常クリック契約](common-inventory-clicks.md)を参照する。


## Shift転送と共通source型

両mode handleの`transfer_inventory(InventorySource, slot)`を使う。
sourceは`Player`のslot 5..45か、元の`Container { screen }`のnative screen slot。
通常PICKUPの`InventoryClickSource`は`InventorySource`と同一型の互換aliasで、importを共通名へ移せる。
装備先やdestination順序はnativeが決めるため、利用側が固定destinationを渡さない。
`inventory_transfer_record()`の`ObservedTransferred`を待ち、全changed slotsのfresh実受信を確認する。
カーソルに通常itemを持っていても転送でき、数量・dataを保持する。転送で変化しないcursorの
ordinalは実受信のまま保持し、新規packet受信を主張しない。cursorだけにdataがある場合も
受信registryで意味を解決する。modernのdata付きcursorはfull再同期を明示的に要求し、Empty比較markerを
実カーソルとして扱わない。数量・data・registryの変化が検出されたらinspectionへ進む。
通常source/destinationの受信NBT/componentsも、元registryの意味と実効容量を検査して使用する。
dataの表現が正規化されても同じnative fieldなら一致する。modernの変更した`equippable`も
実効装備先とarmorの許可対象から振り分ける。data付き装備済みarmorも取り出せる。survivalは束縛の制限を適用し、creativeは元ゲームのmode判定に従って取り出す。modernの制限効果は受信enchantment定義に従う。
旧native `OperationAdmissionError`を網羅matchする利用側には
`BoundedInventoryTransferInProgress`のarm追加が必要になる。
詳しくは[Shift転送契約](common-inventory-transfers.md)を参照する。

## cursor付きcloseと取消後の処理

`close_container(screen.id)`は、実cursorがitemでも十分な既知player main/hotbar容量があれば、返却してから閉じる。
従来の「nonemptyなら即エラー」を使って呼出側で返却する分岐は不要になる。未知cursor、未解決操作、未検証画面、
容量不足等は送信前にエラーになる。通常itemの受信NBT/components付きcursorも、
元registryで意味を解決し、実効容量で計画して返却する。未解決dataや特殊override・bundle内部収納は追加対応を要する。

`ContainerCloseStage::ReturningCursor`を追加したため、stageを全分岐している利用側はこのvariantへ対応する。
`return_plan`はI/O前の返却先とPredicted値、`return_steps`は各PICKUPの実before・送信・実結果を持つ。
各stepの`InventoryClickId::close()`は親close IDを返す。通常の`inventory_click_record()`は上書きしないため、
close後の現在cursorは`player_state()`または親recordの最後の実`cursor_receipt`で確認する。

両版とも待機側の取消後もowned返却→実受信→closeは継続し得る。同じscreenのcloseを再呼出して復旧しない。
`container_close_record()`はwriter待ち中も進捗を返し、timeout・途中の実変化・配信不確実性は最初の理由をinspectionへ残す。
各返却stepは完全送信後5秒以内の実source/cursorを要求し、legacyはmatching実比較応答も必要。
未解決recordや値の復元から再試行の許可を作らず、実状態を調査して新しい接続/画面から明示的に再開する。

`dispatched`は全量CLOSEを書いた事実で、返却stepだけの送信やサーバーACKではない。
`server_close_sequence`は元openingへの新しい実CLOSEだけを示す。無応答の正常vanillaでも`None`のままでよい。
実positionの同値再受信は履歴ordinalを保存しつつ許可し、modernはteleport確認/position応答の完全送信より前に新しい位置を
通常操作へ公開しない。契約・native証拠は[共通close](common-container-close.md)を参照。


## Item dataの受信拡張

`ItemData::ModernComponents { patch }`、`ItemComponent`、`ItemComponentPatch`と
`ItemComponentDefinition`を追加した。modernの対応する非default stackは、欠測の代わりに
元の実受信dataを持つ`SlotKnowledge::Item`になる。追加値bytesと明示的な削除を保持し、
prototypeが空や、default-only操作に使えるとは扱わない。
`Default`・`LegacyNbt`の既存値はそのまま。非対応complex componentは引き続き欠測。

`RegistryKind::ItemComponent`を追加したため、このenumを全分岐している利用側は対応を要する。
`Registry::item_component` / `item_component_by_native_id` / `item_component_definition`で
版とnamespaceを保持した型を取得する。legacy NBTへcomponent IDを流用しない。

native modern拡張の`operations::InventorySlot`には`ItemWithComponents`が加わる。
既存の`Item { item: PlainItem }`はcomponent-free stackを表す。
全分岐を追加し、`ItemWithComponents`を`Item`やEmptyへ落とさない。
native default SWAP/default cursor hashは非default patchを送信前に拒否する。
対応範囲・原codecの証拠・次の操作対応は[共通item data](common-item-data.md)を参照。

### レシピ受信

`Client::received_recipes()`で、両版のrecipe情報を同じ型から参照できる。
`entries()`・`display()`・`requirements()`・`unlocked()`で受信内容を確認し、
`output_items(&catalogue)`で表示結果のitem候補を探す。1.16.1は全宣言とbook状態、
1.21.11は解放済みdisplay entriesを受信するため、受信していない情報は補完しない。
旧版のrecipe名や新版のnumeric IDは、共通入口ではopaqueな`RecipeId`へ移行する。
旧版の解除後の宣言保持と新版の削除・再追加のidentity変更を区別する。
レシピ表示は製作permissionや在庫ではなく、recipe計画・配置は後続対応となる。
詳しくは[共通レシピ受信](common-recipes.md)を参照。

### 製作入力と受信表示

`Client::received_crafting()`は両版共通で、playerの2×2または開いているcrafting tableの3×3を返す。
ほかのUIや未確立のplayer UIでは`None`。グリッドの寸法、入力座標とscreen slotの対応は
選択版の元menuコンストラクタから取得したデータを使用する。

`ReceivedCrafting::input(x, y)`は受信済み`ReceivedSlot`を返す。未受信の`None`、受信したEmpty、
Item、Unavailableは別々に保持する。`registry_state()`と各slotのregistry ownerは同じcapture境界に固定され、
従来cacheやクリック予測を入力として使用しない。playerの`SubmittedClose`も`source()`で明示し、
新しいplayer OPENを受信したことにはしない。tableはsession/world/開き直しを識別する`ScreenId`を保持する。

`input_source(x, y)`が返す`(InventorySource, u16)`を、受信modeに一致するSurvival/Creativeの
`click_inventory`へ渡して材料を置く・戻す操作を行う。結果のslot 0は通常PICKUPへ渡せない。
`result()`はサーバーから受信した表示だけを返し、材料消費や製作完了を意味しない。
新版のinput base capacityは99、旧版は64で、操作時には各itemの有効最大数との小さい方を使用する。

`Survival/Creative::take_crafting_result(&received_grid)`は、実受信の空cursorと現在も一致する
封じられたsnapshotを要求し、結果slot 0へ左PICKUPを一度だけ送る。通常の`click_inventory`
では結果slotを扱わない。新版でplayer製作枠が未受信の場合、既知の通常sourceへの
最初のPICKUPでnative full更新を要求し、受信するまで枠をEmptyと推測しない。snapshotのsession・world・registry・opening・入力/result受信境界を
送信前に再確認し、取消後も送信ownerと`crafting_take_record()`を保持する。書き込みが停止しても
記録getterは書き込み完了を待たない。未知の配送状態や最初の競合を固定し、自動再送しない。

`CraftingTakeRecord::cursor_prediction`だけが予測で、材料減算・remainder・次resultの予測はない。
`ObservedTaken`は完全な送信、予測に合う新規実受信cursor、全input/resultが一つの新規full境界で
受信された`after`を要求する。旧版では実際のcomparison replyも必要。両版ともnative full resyncを
要求するが、revision mismatchやEmpty comparisonは受信証拠ではない。表示resultが同じまま再生成
されるレシピも扱い、例えばケーキのバケツを実受信remainderとして保持する。成功した履歴は現在の
在庫と区別する。recipe選択/計画、非空cursorへのresult merge、shift-craftingは後続対応を要する。
旧版の`craft_once` / `take_crafting_result`でローカルに減算した材料は共通APIの受信結果にしない。

### 製作台の開閉

Survival/Creativeの`open_container(target)`は、両版でnative first outlineの製作台も扱う。
実受信の空の両手・cursor、受信mode、健康な停止姿勢と届く距離を確認し、同じ公開APIで
OPEN・46slot full・cursor・新版processing ACKを個別に保持する。`ObservedContents`後の
`received_crafting()`は、元menuから取得した3×3座標とその開き直しを区別する`ScreenId`を返す。

`close_container(screen)`はその開きを一度だけ閉じる。既知cursorがあればnative mappingで
player側のslotを選び、各PICKUPの実source/cursor応答を確認してからCLOSEを送る。
盤面の材料はnative crafting menuのclose処理に任せ、Clientは消去や在庫への加算を予測値で
書き込まない。`dispatched`はCLOSE送信の事実であり、材料が戻ったこと・server closureのACKではない。
閉じる前の材料は`ContainerCloseRecord.initial_screen`の受信状態に保持する。

元サーバーで生存・在庫空きのある条件の材料返却を検証するが、満杯・死亡・切断時などの
材料の処理は同じ保証にしない。必要な材料の保存確認には、その後の実player slot受信や
在庫の調査を用いる。再度開いた画面では新しい`ScreenId`を取得し、古いinput/close要求を再送しない。
結果slotを取る操作、recipe消費・remainder、製作台でのSWAP/QUICK_MOVEは後続対応を要する。

### Recipe-book materials

Use `Client::recipe_book_materials(&RecipeId, crafts, maximum_bound)` for received
main/hotbar material-only assignment. Inspect `ReceivedItem::recipe_book_stock()`
for the native inventory filter rather than using ingredient membership as an
eligibility predicate. This shared API is available in both versions and modes.
See [common recipes](common-recipes.md) for provenance, missing-data errors, batch
semantics and the separate placement/consumption work that remains.

### Coherent crafting context and read-only planning

Use `Client::received_crafting_context()` when recipe, inventory and crafting grid
must share one received boundary. `context.recipe_layout(id)` maps the received
display into the actual player/table UI; it does not pick ingredient stacks.
`context.grid_return_plan()` checks hypothetical main/hotbar/offhand return
capacity and labels all resulting values `Predicted`. Do not dispatch its steps
as click commands or replace actual inventory receipts with its predictions.
Unknown destinations, missing selected-hotbar bases and unresolved item data
produce errors. A resolved local selection stays `Submitted` in
`plan.selected_hotbar()`; it never becomes a server acknowledgement. Positive
native counts above ordinary limits remain known. Native fixed-destination
leftovers appear in `unreturned_splits()` and prevent `fits()`, even when another
slot is empty. The cursor requires its own return handling; it is excluded from
this grid-only capacity simulation. See [common recipes](common-recipes.md).

For coherent recipe-book planning, obtain `Client::received_crafting_context()`
and call `recipe_placement_plan(recipe.id(), RecipePlacementAmount::Next)` or
`Maximum` on that context. Next can increase an existing matched grid; it does
not mean a fixed exact batch or one output item. Inspect grid matching, native
material counts, full return capacity and compatible post-return sources together.
`can_place()` is historical preflight, not an operation reservation or server ACK.
Planning emits no recipe packet. Owned dispatch/actual placement receipts are
still in progress; do not replace consumer placement with a successful plan alone.


## 共通の採掘復旧とログインidentity

両版は`client.survival().prepare_mining_profile_recovery(MiningId)`から明示的なcloseと
一度だけの`reconnect(config, MiningRecoveryTarget)`へ進めます。戻り値の共通
`RecoveredSurvivalClient.client`を使い、旧IDや旧接続へ次のmutationを送らないでください。
modern専用の`checked_survival`の復旧入口・証拠型は維持します。
同じ名前の共通型とnative診断型は契約が違うため、必要なimportを明示します。

共通`MiningRecord`にも`recovery_attempt`が加わります。struct literal・保存schemaを更新し、
claimと復旧成功を区別してください。`MiningRecoveryMethod/Target/Attempt`の純粋な値型は
共通側の所有へ移し、modernの旧importは同じ型のre-exportです。
新しい`Client::connection_identity()`は実受信UUID/nameと現在のsessionを返します。
legacyの試験serverは空のLOGIN_SUCCESSを送らず、16byte UUIDと元のprofile名を送る必要があります。
欠損・名前不一致・余分なfieldはlogin時に拒否します。
詳細とvanilla限定条件は[採掘後の復旧](common-mining-recovery.md)を参照してください。

## 共通の記録・再生・scene

`Client::stop_packet_trace()`の戻り値は`voxrig::client::PacketTrace`へ移ります。
modernの旧`PacketRecord/PacketTrace` importは同じ共通型のre-exportです。
`PacketRecord.phase`は文字列から`PacketPhase::{Configuration, Play}`へ変更します。
JSONでは引き続き`configuration`／`play`ですが、Rustの比較・struct literalは更新してください。
position decoderのlocal入力、stop時のclient frameが記録fieldに加わります。
旧JSONの追加fieldは既定値で読み込めても、再生に必要なlocal基準が欠けた記録は拒否します。

両版で接続開始から再生する場合は`Client::connect_recorded(config, maximum_bytes)`を使い、
`stop_packet_trace`で終了した`PacketTrace::replay(region, maximum_chunks)`を呼びます。
途中開始の`start_packet_trace`は診断区間用です。
戻り値のinventoryは`RecordedSlotKnowledge/RecordedItemStack/RecordedItemData`という診断型です。
保存値を操作用のslot/registry/screenへ変換する移行は行わず、live Clientから新しく取得してください。

限定sceneは`client.survival().capture_scene(region)`と`scene.preview_path(controls)`へ移せます。
有限地上歩行とsceneの既存APIは、登録された乾いたstairs/slabも扱います。
受信した全propertiesで版の元形状へ照合し、waterloggedや未知形状は拒否します。
追加の版別分岐や新しい移動入口は不要です。範囲は[共通dry terrain](common-dry-terrain.md)を参照。
返る共通`ScenePreview`は操作planではありません。modern専用の編集・連鎖・assumed scene等は
従来の版固有入口を維持し、Bでより広い共通契約を検討します。
詳細は[記録・再生・scene](common-recording-scenes.md)を参照してください。

## 共通scoreboardとClientManager

scoreboardは`Client::scoreboard_state()`へ移行できます。値ごとの元ordinalを保持し、
objective/display/score/resetを扱います。表示方式は共通enumで、textとmodern number formatは
元のnative dataを保持します。完全なserver catalogue・renderer・全UiStateの共通化ではありません。
legacyの既存UiStateは維持し、空objective名のowner resetを修正しました。

複数接続の基本lifecycleは`ClientManager::new(capacity)`と`connect(name, config)`／
`get(name)`／`disconnect(name)`／`shutdown()`へ移行できます。
旧BotManagerのserver-global setupを使わず、Clientごとにconfigを指定してください。
manager生成とplay readinessは別です。shutdownはterminalで、外部にあるcloneも閉じます。
event集約・physics metrics・shared chunk storage等の版固有入口は維持し、より広い移行はBへ残します。
詳細は[共通UIとmanager](common-ui-manager.md)を参照してください。

乗車関係は版固有`vehicle()`／passengers cacheの走査から`Client::vehicle_state()`へ移す。
未受信と実下車を区別し、spawn位置を車両の現在位置として扱わない。
版固有raw dismountは、実受信の`MountId`をmatching mode handleの`dismount(mount)`へ渡す形に移す。
`Client::dismount_record()`で元の実除外を確認し、`ObservedUnmounted`後に同じhandleから
`complete_dismount(record.id)`を一度呼ぶ。要求とneutralを続けて送らず、取消時にも再送しない。
`Completed`は地上支持・立位の確認ではないため、地上操作の許可を戻す条件に使わない。
操縦・車両physics・広い下車後継続はBに残る。
契約は[共通乗車関係](common-vehicles.md)を参照。
