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

## 採用検証

Voxrig commit、利用側commit、ゲーム版、Rust toolchain、実行コマンドと結果を記録します。
deepplanning: coherent observation、context付き操作、製作・かまど・採取、取消・切断。
minetool: イベント過多、閉じたwindowの遅延更新、作物掘削、原木から石ツルハシ製作。
DustRoute: native observation、piston/recovery、照準、配置・除去・取消・recording。
各環境で成功した同じVoxrig commitをmainの統合候補とします。
