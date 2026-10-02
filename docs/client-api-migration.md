# 0.2 client APIへの移行

利用側のソース公開は不要です。各環境で以下の移行を行い、同じdevelop commitを固定して検証します。

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
vendorの内容をdevelopの固定commitで置き換えるか、同じcommitをGit dependencyの`rev`へ指定します。
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
採掘ではair受信後も元接続のmutationは保留され、明示的なretirementと別接続の
新しいUUID削除受信を確認してからfresh recoveryを行います。
移動は有限の`SurvivalInput`列と別接続のobserverを渡し、予測・観測・誤差を区別します。
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
`Client::survival()`から取得し、型を`voxrig::checked_survival`からimportできます。
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

## 採用検証

Voxrig commit、利用側commit、ゲーム版、Rust toolchain、実行コマンドと結果を記録します。
deepplanning: coherent observation、context付き操作、製作・かまど・採取、取消・切断。
minetool: イベント過多、閉じたwindowの遅延更新、作物掘削、原木から石ツルハシ製作。
DustRoute: native observation、piston/recovery、照準、配置・除去・取消・recording。
各環境で成功した同じVoxrig commitをmainの統合候補とします。
