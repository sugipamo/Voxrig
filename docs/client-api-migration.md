# 0.2 client APIへの移行

利用側のソース公開は不要です。各環境で以下の移行を行い、main上の固定commitまたはreleaseを基準に検証します。

## deepplanning派生版から

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
| `client.java_1_21_11_operations()`でのcreative基本操作 | `client.creative()`。`set_creative_hotbar`→`set_hotbar`、`dig_creative`→`break_block`。戻り値はDispatchReceipt |
| PlayerStateを共通playerとして使用 | `client.player_state()`のPlayerObservation。追加検査契約のnative PlayerStateとは区別 |
| 版なしの`item_id`/`item_name`など | `client.registry()`。整数IDはversion/kind付きRegistryIdとして保持 |
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
