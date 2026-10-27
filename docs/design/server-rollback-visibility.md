# サーバーによる巻き戻しの上位への可視性（調査）

調査日 2026-10-27、対象 `main` @ `972c6f3`（ns-01〜08、#279、#286 マージ後）。コードを静的に読んだ調査であり、ビルド・テスト実行・LIVE 再現は行っていない。

**目的**: サーバーが状態を巻き戻す・覆す場面を洗い出す。上位（Golemkit など公開 API の呼び出し側）が知っておくべきなのに、Voxrig の公開 API が適切に返せていないものを特定する。

**表記**
- 「確認」はコード上で経路を読んだもの。「推測」は vanilla サーバーの挙動、または経路が見つからなかったことに依存するもの。
- 行番号は `972c6f3` 時点。
- 「16」は `src/versions/java_1_16_1/`、「21」は `src/versions/java_1_21_11/` の略。

**上位からの見え方の分類**
- **見えない**: 公開 API・通知のどこにも出ない。
- **遅れる**: 最新値の再照会や汎用通知（`PlayerChanged`・`BlocksChanged` など）でしか分からない。原因・対応する操作との結び付きがない。
- **別の意味で返る**: Pending・Running・Accepted・External・Timeout など、巻き戻しとは別の意味の値として返る。
- **適切**: 型付きの結果や通知として、巻き戻しだと分かる形で返る。

## 横断的な所見

1. **「何も起きなかった」拒否は、ほぼすべて Pending の行き止まりになる。**
   - 対象: 厳密配置（B1/B2）、クラフト取り出し（C1）、レシピ配置（C3）、コンテナを開く（C4）、厳密クリック（I3）、drop/hand-swap（I6/I7）、降車（E2）。
   - サーバーが拒否して元の値を再送しても、「受信値が `before` と等しい」ことは衝突とも完了とも扱われない。期限のない記録は永久に Pending のままになる。
   - 根拠の例: `16/client/common_placement.rs:368-376`、`src/client/inventory/click.rs:719-731`。
   - Voxrig は多くの場合「処理済み境界（1.21.11 の ACK、全量 resync）＋値が不変」という証拠をすでに受け取っている。分類していないだけである。
2. **Voxrig は予測値を状態に書き込まない**（`src/client/inventory/outcome.rs:37-38`、docs/inventory-divergence.md）。このため巻き戻しは「戻る変化」としては現れず、「差分がない」こととして現れる。`inventory_changes_after` は差分がないと何も記録しない（`src/client/inventory/changes.rs:391-392`）。
3. **1.16.1 の `ConfirmTransaction accepted=false` は拒否の信号にならない。** Voxrig は全量 resync を誘発するため、わざと不一致の stack を送っている（`16/client/common_click.rs:109`、`src/client/crafting/take/shift.rs:209-225`）。ほぼすべてのクリックが `Rejected` になる。真の拒否は、送信後の全量境界と `before` を比べて判定するしかない。
4. **Body の監視（ns-08）は 1.16.1 だけである。** 1.21.11 では `action_observation` は Unsupported（`src/body/headless/monitoring.rs:4-11`）。既存の `action_outcome` は、gap・world 変更・timeout・取得エラーを黙って `Accepted` として返す（`src/body/headless.rs:245-256`）。`correction()` が返す `Unknown`（chunk/world 消失）も捨てる（同 :247 は `Corrected` だけを記録）。これは ns-08 が 1.16.1 で直した問題と同じ形である。
5. **PlayerControl のラバーバンドは Running のまま進む。** 移動の拒否・訂正は `corrections += 1` になるだけで、status は `Running` のままである（`src/client/control.rs:725-748`）。上位は `control_record()` を定期的に読んでカウンタの差分を取らない限り、「進んでいる」と「押し戻され続けている」を区別できない。

## 表 1: 位置・移動

| # | ケース | 版 | サーバーで起きること | Voxrig 内部の検知箇所 | 上位に今どう見えるか | あるべき返し方 | テスト / LIVE | 状態 |
|---|---|---|---|---|---|---|---|---|
| P1 | 自プレイヤーの位置訂正・teleport | 両 | PlayerPositionLook（16 0x35、`16/client.rs:6532`）、21 `POSITION`（`21/client.rs:1287`）。teleport confirm は即送信（`16/client.rs:6590`、`21/client.rs:1326`） | 受信 pose と entity history（`16/client.rs:6564-6570`、`21/client.rs:1314`）。16 native の `PositionCorrection` は直近2秒に移動を送った場合だけ出る（`16/physics.rs:376-399`） | **適切（事実のみ）**: `entity_history_after` の `OwnPositionCorrection`（`src/client/entity/history.rs:147,602`）。`ReceivedPlayerSnapshot::correction()` は最新1件だけ（`src/client/received_snapshot.rs:110-140`）。共通 event は汎用の `PlayerChanged` だけ | 実行中の control/motion に結び付けた訂正通知（送った pose と受け取った pose の差分） | `16/physics.rs:568`、`21/client/tests.rs:970` | 部分的 |
| P2 | 移動拒否（moved wrongly/too quickly） | 両 | P1 と同じ packet。teleport とはプロトコル上区別できない | 16 は `CorrectionReason::MovementRejectedOrAdjusted`（`16/physics.rs:200,393-398`、native のみ）。21 は `PositionSubmission.superseded_at`（`21/client/motion.rs:34,76-78`、内部のみ） | **別の意味**: PlayerControl は model を teleport させ、`Running` のまま `corrections += 1`（`src/client/control.rs:725-748`）。送信を続けるので、押し戻されても進捗に見える。tick 間に2回訂正されても1回に潰れる（推測、`src/client/control.rs:316-323`） | owner 単位の有界な訂正ログ（受信 seq、送った/受け取った pose、差分）。連続訂正時の停止・一時停止方針と通知 | `src/client/control.rs:1533` | **open** |
| P3 | 有限 survival motion / 経路中の訂正 | 両 | P1、自分への velocity、爆発 | 16 `interrupt_common_motion_operations` → `RequiresInspection`（`16/client/common_motion.rs:362-405`）。21 `stable_context`（`21/client/operations/movement/control.rs:151-197`） | **遅れる**: `motion_record()` の status と自由文 `problem` を再照会するしかない。event・型付き原因・訂正後 pose はない | 型付きの中断原因（Correction{seq,pose}/Velocity/Explosion/World）と通知・待機 API（旧 09 と重なる） | `21/client/operations/mining/prediction_tests.rs:123`。LIVE `survival-motion-live-20261002.json`（訂正ケースは含まない） | **open** |
| P4 | Body `Move` の訂正 | 両 | P1 → Body 証拠 `Update::Pose`（`16/client/body.rs:566-586`、`21/client/operations/body.rs:22-35`） | `correction()` → `Corrected{Pose}`（`src/body/headless.rs:1199-1219`） | 受理窓内は**適切**。Accepted 後の訂正は 1.16.1 なら `action_observation` で取れる。1.21.11 は Unsupported。held lease（`ticks: None`）は Accepted 後に証拠を見ずに sleep する（`src/body/headless.rs:1133-1140`）。Move ループは `Paused` を無視し、`Stopped` だけを見る（`src/body/headless.rs:1048-1053`） | 監視を 1.21.11 にも移植。lease が上書きされたことの通知 | LIVE `body-api-live-20261024.json`（21 teleport_rollback=corrected）、`body-send-trust-live-20261025.json`（両版 late teleport）、`body-monitor-live-1.16.1-20261027.json.gz` | 1.16.1 **ns-08 で修正済み**。1.21.11 は部分的 |
| P5 | best-effort 接触の原点が訂正で無効化 | 両 | P1 | `apply_pose` → `omit_contacts_at(.., Correction)`（`src/client/control.rs:731-741`） | **適切**: `control_changes_after` | — | LIVE `control-degradation-live-1.16.1-20261027.json.gz` | **ns-05 で修正済み** |
| P6 | 乗り物の位置訂正 | 両 | 16 0x2c（`16/client/play_entities.rs:38-71`）、21 `VEHICLE_MOVE` | 16 は `vehicles.interrupt_motion` と乗り物 entity の pose 更新。21 は `interrupt` と `interrupt_motion`（`21/client/operations/survival.rs:160-183`）。21 の entity 側に pose デコード経路あり（`21/client/entity.rs:290-314`）。公開まで届くかは未確認 | **遅れる**: `vehicle_state().motion_correction_sequence` だけ（`src/client/vehicle.rs:75`）。乗り物 control は次の step を Unavailable で拒否する（`src/client/vehicle/control.rs:511-514`）。共通 event はない（`16/client.rs:6888` の `_ =>`） | `VehicleCorrected{seq,pose}` の通知・ログ | `16/client/common_api.rs:1349`、`21/client/outbound/tests.rs:3341` | **open** |
| P7 | 強制回転 `PLAYER_ROTATION`（0x47） | 1.21.11 | `21/ids.rs:123` | **処理なし**。既知 ID の no-op（`21/client.rs:1510`）。ほかに参照なし（grep で確認） | **見えない**。次の移動 packet でローカルの yaw/pitch が上書きする（推測） | デコードして受信回転として適用。Body 証拠と Aim 通知 | なし | **open（新規。旧 16 は FACE_PLAYER/CAMERA）** |
| P8 | 強制視線 `FACE_PLAYER` | 両 | 16 0x34（`16/client/play_player.rs:227-256`）、21 は no-op | 16 は yaw/pitch を計算して `PlayerChanged` | 16 は汎用通知だけ、21 は見えない | 旧 16 を参照 | — | open（旧 16） |
| P9 | 自分への velocity 上書き・ノックバック | 両 | 16 0x46（`16/client/play_entities.rs:212-237`）、21 `ENTITY_VELOCITY` | 16 `own_velocity_receipt` と有限 motion の中断（同 :223-228）。21 `local_player.velocity` | **ほぼ見えない**: `PlayerObservation`（`src/client/observation.rs:296-340`）にも受信 snapshot にもない。PlayerControl は `velocity_updates` カウンタだけ（`src/client/control.rs:750-756`）。21 は `EntityUpdated{self}` という別の意味の通知になる（`21/client/event_kinds.rs:80`） | player 観測に自 velocity の受信値、history に `OwnVelocity` | `src/client/control.rs:1615,1636`、`21/client/outbound/tests.rs:264` | **open** |
| P10 | 爆発ノックバック（自分） | 両 | Explosion | 16 は native `motion.velocity` に加算（`16/client/play_world.rs:162-165`）。21 は Ground owner なら一度だけ加算、それ以外は Stopped（`21/client/operations/continuous.rs:355-374,453-456`） | 事実は `explosions_after` の `player_knockback` で取れる。**16 は実行中の PlayerControl が毎 tick `self.motion` を上書きする**（`16/client/common_control.rs:614-625`）。上書きは確認、impulse が失われることは推測。21 は ControlRecord に計上しない | 16 は 21 と同様に適用するか、理由付きで停止。impulse を計上 | `21/client/outbound/tests/explosion.rs:197` | 16 open、21 部分的 |
| P11 | 死亡・リスポーン・次元変更 | 両 | 16 0x3a（`16/client/play_player.rs:275-396`）、21 `RESPAWN` | generation 更新と中断。Body は `Update::World` | **適切（通知）**: `WorldChanged`。control は Stopped で、再照会しないと分からない（旧 09）。Body は `Unknown "world lifetime changed"`（`src/body/headless.rs:1244-1247`） | control 状態変更の通知（旧 09） | LIVE `common-respawn-20261007.json` | 部分的（旧 09 は open） |

## 表 2: インベントリ・スロット

| # | ケース | 版 | サーバーで起きること | Voxrig 内部の検知箇所 | 上位に今どう見えるか | あるべき返し方 | テスト / LIVE | 状態 |
|---|---|---|---|---|---|---|---|---|
| I1 | クリック・transfer が無効果（executor API） | 1.16.1 | ConfirmTransaction(false)、WindowItems、SetSlot(-1,-1) | `inventory_protocol_response` → `rejected()`（`16/client/inventory_execution.rs:310-326`）、全量境界で `Comparison::inspect`（`src/client/inventory/outcome.rs:158-221`） | **適切だが曖昧**: `Diverged` と `causes` に `Rejected` が載る。ただし `Rejected` は全クリックに付く（所見3）。無効果は `divergence.actual` が空であることから推し量るしかない（`src/client/inventory/outcome.rs:233`）。`inventory_changes_after` には何も出ない | `InventoryDivergence` に `NoEffect` / `PartialOrDifferent` の分類 | `src/client/inventory/divergence_tests.rs:142`、`16/client/inventory_execution.rs:1060-1088`。LIVE `inventory-divergence-20261026.json`（真の無効果拒否は含まない） | 部分的（#269/#270/#286） |
| I2 | 同上（stateId 不一致 → 全量 resync） | 1.21.11 | 適用後に ContainerSetContent(stateId) を送る。拒否ビットはない | 同じ `inspect`。21 には `rejected()` 呼び出しがない | **別の意味**: `Diverged` で、原因が無いため `causes=[External]` に落ちる（`src/client/inventory/outcome.rs:226-230`）。自分の操作が無効果だったのに「外部変更」と表示される | I1 と同じ。「観測上の効果なし」と明示 | `src/client/inventory/divergence_tests.rs:204,260`（合成）。LIVE は成功経路のみ | **open** |
| I3 | 厳密クリック記録（`click_inventory`）と helpers が無効果 | 両 | I1/I2 と同じ | `src/client/inventory/click.rs:719-731`: `before` と等しい受信は receipt にも衝突にもならない。RequiresInspection になるのは executor の ResyncRequired のときだけ（`21/client/finite_work.rs:63-89`） | **別の意味**: 記録は `Pending`。helpers は3秒後に `Timeout "click on slot N not received"`（`src/client/inventory_helpers.rs:201-209`）。無効果なのに packet 欠落に見える | executor が Diverged で終わったら、厳密記録を「全量境界で不変」で終端にする。helpers は型付きの無効果エラーを返す | `src/client/inventory/click.rs:966`（衝突のみ） | **open**（推測含む） |
| I4 | 旧 native `click_slot_and_wait` | 1.16.1 native | ConfirmTransaction(false) | `16/client/play_inventory.rs:36-41` → `16/lifecycle.rs:1403-1407` | **別の意味**: ACK さえ来れば適用の有無に関係なく `Ok`（`16/client.rs:4545-4546`）。native `WindowTransaction{accepted}` は共通 event に写像されない | 「プロトコル ACK のみ」と明記するか、効果の比較を返す | なし | open |
| I5 | サーバーによる持ち替え（選択スロット） | 両 | 16 HeldItemSlot 0x3f、21 `HELD_ITEM_SLOT` | `16/client.rs:5963-5971`、`21/client/operations.rs:1377-1388`（`from_server:true`） | **遅れる**: `InventorySnapshot` に selected_hotbar がない（`src/client/inventory/changes.rs:66-79`）。保留中の hand swap/drop は RequiresInspection になる（`src/client/inventory/hands.rs:213-221`、`src/client/inventory/drop.rs:240-246`）。`select_hotbar` は Submitted のまま | 選択スロットの before/after ログ（Submitted と Received の出所付き） | なし | open（旧 11） |
| I6 | selected drop が無効果 | 1.21.11 共通（16 共通は Unsupported） | resync 用のクリックに対する全量応答 | `21/client/operations/inventory/drop.rs:87-100`、契約 `src/client/inventory/drop.rs:286-306` | **見えない・詰まる**（推測）: 期限がなく `Pending` のまま（`21/client/finite_work.rs:106-115`）。未解決の間はほかの操作を塞ぐ（`21/client/operations.rs:1095-1102`）。自分の drop による減少は `External` に分類される | resync 境界後に `ObservedUnchanged` で終端。自操作の差分を区別 | `21/client/operations/inventory/drop_tests.rs:22,218,526`（無効果ケースなし）。LIVE `common-selected-drop-20261022.json`（成功のみ） | **open** |
| I7 | swap hands が無効果 | 1.21.11 共通 | 何も来ない、または SetSlot | 契約 `src/client/inventory/hands.rs:262-277`。resync も期限もない | **見えない・詰まる**（推測）: `Pending` のまま | resync と期限を付け、型付きの不変結果を返す | `21/client/operations/inventory/hand_swap_tests.rs:57,230`。LIVE `common-hand-swap-20261022.json`（成功） | **open** |
| I8 | creative スロット書き込みの拒否 | 両 | 成功時の echo の有無は版で異なる（推測） | 16 は送信前にスロットを `None` に（`16/client/common_api.rs:349-351`）。21 は `Unavailable` と `pending_creative`（`21/client/operations.rs:716-721`） | **遅れる・区別不能**: `DispatchReceipt` だけ。成否にかかわらず、次の resync まで不明。21 は `pending_creative` が配置・採掘・インベントリ操作を塞ぐ | creative 書き込み記録（resync probe と型付き結果） | `16/client/common_api.rs:2802`、`21/client/outbound/tests.rs:1247` | open |
| I9 | use・配置・drop による消費の差し戻し | 両 | SetSlot / WindowItems | ledger だけ | **別の意味**: executor 送信以外の差分は `External` に分類される（`src/client/inventory/changes.rs:408-410`） | 自分の use/place/drop 窓に入った差分に印を付ける | なし | open |
| I10 | foreign close 後のカーソル・クラフト | 両 | CloseWindow と resync | `16/client/play_inventory.rs:95-180` | **適切**: ResyncRequired・inspection | — | #279 の LIVE `pr279-crafting-*-1.16.1-20261027` | **#279/#286 で修正済み** |

## 表 3: ブロック設置・破壊

| # | ケース | 版 | サーバーで起きること | Voxrig 内部の検知箇所 | 上位に今どう見えるか | あるべき返し方 | テスト / LIVE | 状態 |
|---|---|---|---|---|---|---|---|---|
| B1 | 厳密 `Survival::place_cube` を即拒否（保護、entity が占有など） | 1.16.1 | BlockChange 0x0B で target=元の値。SetSlot なし | `16/client/common_placement.rs:351-389`。`before` と等しい受信は衝突にしない（:368-376）。material は `after` とだけ比較（:276-285） | **別の意味**: 記録は永久に `Pending`。Rejected 段階がない。解放は完了時だけ（:304-326、`16/lifecycle/bounded_motion.rs:1197-1202`）。それまで look/select/move/次の配置が拒否される（`src/client/tests.rs:1318-1327`）。docs/survival-placement.md:37-40 に既知として記載 | 送信後の新しい target 受信が `before` に等しく support も不変なら、`ServerReverted` で終端にして owner を解放する | 拒否のテストなし。LIVE なし | **open（高）** |
| B2 | 同上、ACK あり | 1.21.11 | BlockUpdate(target, 元) の後に BlockChangedAck(seq ≥ intent) | `21/client/operations/placement.rs:851-890`。完了には ack・target・material がすべて必要（:752-783） | **別の意味**: `Pending`（`processing: Some`、`target_receipt: None`）で、lock は保持したまま。native の `wait_survival_placement` がタイムアウトすると `ledger.fail(TransactionExpired)` になり（`21/client/operations/placement.rs:689`）、**接続全体が terminal になる** | 新しい ACK が来て target がまだ `before` なら「target 拒否」で終端にして解放する。タイムアウトで接続を壊さない | `21/client/operations/placement/tests.rs:335`（ACK 欠落のみ） | **open（高）** |
| B3 | `ObservedPlaced` の後に差し戻し | 両 | 後続の BlockChange | 配置後は無視（`16/client/common_placement.rs:359`、`21/client/operations/placement.rs:859`） | 記録は凍結された履歴（設計どおり、`21/client/operations/placement/tests.rs:513-521`）。汎用の `BlocksChanged` だけ | 履歴としては可。受信差分ログ（旧 12） | `21/client/operations/placement/tests.rs:485-533` | open（旧 12） |
| B4 | Pending 中の衝突（いったん置かれてから戻る、support 消失、dig ACK 訂正、chunk 置換） | 両 | BlockChange / dig ACK / chunk | `16/client/common_placement.rs:15-22,341-349,376-385`、`21/client/operations/placement.rs:836-849,866-882` | **適切**: 理由付きの `RequiresInspection`。後で元に戻っても消えない | — | `16/client/common_placement.rs:542`（`digging_ack_correction_reaches_pending_placement`）ほか | **ns-02 で一部修正済み** |
| B5 | 確認前に消費された material を差し戻し | 両 | SetSlot / WindowItems | `16/client/common_placement.rs:276-285`、`21/client/operations/placement.rs:811-819` | **適切**: `RequiresInspection` | — | `16/client/common_placement.rs:744-793` | ok |
| B6 | material が減らない、または完了後に戻る | 両 | SetSlot | 該当規則なし（推測）。完了後の receipt は凍結（`21/client/operations/placement.rs:946-952`） | 完了前は永久 Pending。完了後は見えない（`inventory_changes_after` に帰属なしで出るだけ） | material の結果を独立したフィールドに（Consumed / NotConsumed / Restored） | なし | open |
| B7 | Body `PlaceBlock` の差し戻し | 両 | 期待と矛盾する BlockChange | `src/body/headless.rs:1183-1198` | **適切**: `Corrected{Block}`。Accepted 後は 1.16.1 だけ（ns-08）、1.21.11 は Unsupported | 監視を 1.21.11 に移植 | LIVE `body-api-live-20261024.json:67`、`place-activation-live-20261025.json:299,623`、`body-send-trust-live-20261025.json` | 1.16.1 **ns-08 で修正済み**、1.21.11 の Accepted 後は open |
| B8 | Body `PlaceBlock` の material 差し戻し | 両 | SetSlot | 監視なし（`expected()` は target だけ、`src/body/headless.rs:801-838`） | **見えない** | material の期待値と訂正 | なし | open |
| B9 | 掘削の拒否・差し戻し | 1.16.1 | AcknowledgePlayerDigging(successful=false) と状態 | `16/client/play_world.rs:6-77` → `BlocksChanged` と `LegacyDiggingAcknowledged`（`src/client/events.rs:132-141`） | 通知は**適切**（ns-02）。ただし `DigRecord` に拒否フラグがない（`src/client/dig.rs:24-36`）。`Survival::dig` は拒否でもタイムアウトでも同じ `removed:false` を返す（`src/client/operational_dig.rs:591-627`） | ACK の判定を `DigRecord` / `DigResult` に載せる | `16/client/digging_ack_notification_tests.rs:14,131`。LIVE `digging-ack-live-1.16.1-20261027.json.gz` | **ns-02 で修正済み**（DigRecord は部分的） |
| B10 | 掘削の拒否・差し戻し | 1.21.11 | BlockUpdate(元) と BlockChangedAck(seq)。成否フラグなし | ACK は sequence だけ（`21/client/operations.rs:1464-1473`）。`mining_received`（`21/client/operations/mining.rs:756-789`） | 共通 mining は**別の意味**: `PendingAfterFinish` と非 air の `target_receipt`（推し量れるが明示されない）。`Survival::dig` は B9 と同じ。Body `BreakBlock` は**適切**（`Corrected`） | 「処理済みで target 不変」の結果。ACK を成功に格上げしない | 明示的な拒否テストなし。LIVE `dig-estimate-live-1.21.11-20261025.json.gz`（時間計測のみ） | **open** |
| B11 | `use_on_block` / `use_item`（バケツ、火打石など）の差し戻し | 両 | BlockChange ×2 と SetSlot | 汎用の `BlocksChanged`、`inventory_changes_after` | **遅れる・帰属なし**: 契約上 dispatch のみ（docs/common-item-use.md:36-42）。21 は `DispatchReceipt.interaction_sequence` を返すが、BlockChangedAck は公開されず、いつ確定したか分からない | `interaction_sequence` をキーにした ACK・処理済み receipt の公開 | `16/client/use_on_block_expect_tests.rs`、`21/client/operations/mining/use_on_block_expect_tests.rs`。LIVE `use-on-block-expect-live-20261025.json`（成功のみ） | open |
| B12 | Body `UseItem` のインベントリ resync | 両 | WindowItems | `src/body/headless.rs:1222-1237` | **適切**（`Corrected{InventoryResync}`）。ただし無関係な window の WindowItems でも Corrected になる（過剰報告、推測。`16/client/body.rs:583`、`21/client/operations/body.rs:43`）。ブロック側の効果は監視しない | resync を window 0 に限定 | `16/client/body.rs:841` | 部分的 |
| B13 | 看板編集の拒否 | 1.16.1 native | 黙って無視（推測） | `Bot::update_sign` は送信のみ（`16/client.rs:4954-4969`） | **見えない**。共通 API にない | 共通の看板 API（期限付き記録） | なし | open（低） |
| B14 | slab / stair / fluid-slab / replaceable 配置の拒否 | 1.21.11（16 は送信前に Unsupported） | B2 と同じ（stair は隣接更新も） | B2 と同じ latch（`21/client/operations/placement.rs:799-882`） | 衝突は RequiresInspection（ok）。純粋な拒否は `Pending`（B2 と同じ） | B2 と同じ | `21/client/operations/placement/tests.rs:1038,1525,1781`。LIVE `checked-*-20261023.json`（成功のみ） | **open** |

## 表 4: クラフト・かまど・コンテナ、Accepted 後に覆る結果

| # | ケース | 版 | サーバーで起きること | Voxrig 内部の検知箇所 | 上位に今どう見えるか | あるべき返し方 | テスト / LIVE | 状態 |
|---|---|---|---|---|---|---|---|---|
| C1 | クラフト結果の取り出しが拒否・無効果 | 両 | 16 は ConfirmTransaction(false)（取り出しでは常に false、所見3）と全量 WindowItems で grid/cursor 不変。21 は fence 後の全量 ContainerSetContent（`21/client/operations/inventory/crafting.rs:149`） | `src/client/crafting/take.rs:509-562`。cursor に出力があるときだけ receipt を立てる（:519）。shift は増分不足で None（`src/client/crafting/take/shift.rs:195`）。`ready()`（`src/client/crafting/take.rs:161-169`）は真にならない | **別の意味**: `CraftingTakeStage` は永久に `Pending`。crafting 送信は常に `Diverged`（`src/client/inventory/outcome.rs:224` が `!self.crafting`）なので、拒否と成功を区別できない。`craft_once` は **Pending のまま Ok を返す**（`src/client/inventory_helpers.rs:429-434`）。`craft_once_checked` は全量 resync で「不適用」が証明されているのに `OutcomeUnknown{Timeout}` を返す（同 :499-501） | 送信後の全量境界（grid・cursor・inventory）が `before` と等しければ `NotApplied` で終端にする。`craft_once` は Pending で Ok を返さない | 拒否経路のテストなし（近いもの: `src/client/crafting/tests.rs:427,487`）。LIVE なし | **open（高）** |
| C2 | 取り出し完了後に出力が resync で消える | 両 | 後続の SetSlot/WindowItems（plugin・dupe 対策、推測） | 記録は不変（`src/client/crafting/take.rs:121-122`） | **遅れる・結び付きなし**: `inventory_changes_after` の External だけ | 変更ログから take id への参照 | なし | open（低） |
| C3 | レシピ本による配置の拒否 | 両 | 何も来ない、または GhostRecipe（推測） | `src/client/crafting/dispatch.rs:425-431,468-470` は `after` を立てない | **詰まる**: 期限がなく `Pending`（`16/client/common_recipe_placement.rs:153-163,263-288`、`21/client/operations/inventory/recipe_placement.rs:75-77`）。ghost は `ObservedGhost` で適切 | 期限か fence を設けて `NotApplied` / `Unknown` | `src/client/crafting/dispatch/tests.rs:141,209`。LIVE `common-recipe-placement-20261006.json`、`common-recipe-ghost-20261006.json`（成功と ghost のみ） | **open** |
| C4 | コンテナを開くのが拒否される（画面が開かない） | 両 | OpenScreen が来ない。21 は BlockChangedAck だけ来る | `src/client/container/open.rs:287-357`。21 の ACK は保持するが、完了にしか使わない（:97,347-348） | **詰まる**: `Dispatched` のまま（:281-285）。Body `ActivateBlock` は **Accepted**（`src/body/mod.rs:170` に「menu が開いたことではない」と明記） | 21: sequence を覆う ACK があって画面がなければ `NoScreenObserved`。16: fence または静穏窓の後に同様 | `21/client/operations/inventory/tests.rs:2134,2235`、`16/client/common_container.rs:879`（衝突のみ）。LIVE なし | **open** |
| C5 | サーバーが強制的に閉じる・foreign close | 両 | CloseWindow | 16 `16/client/play_inventory.rs:179` → `ScreenChanged`（`16/client.rs:6844-6847`）、21 `21/client/event_kinds.rs:65-66`。実行中の送信は `ResyncRequired`（`src/client/inventory/executor.rs:276-283,393-401`） | **遅れる・汎用**: `ScreenChanged` と最新値だけで、「サーバーが閉じた」という型付きの原因がない | 原因（server/local/superseded）付きの画面変更ログ | `16/client/common_inventory.rs:948`、`21/client/operations/inventory/tests.rs:285`。LIVE `pr279-crafting-*-1.16.1-20261027` | **#279 で一部修正済み** |
| C6 | 通常のクリック・transfer の拒否（かまど・チェストもこの経路） | 両 | I1/I2 と同じ | I1/I2 と同じ | I1/I2 と同じ（かまど専用の操作はない。`src/client/furnace.rs` は観測と PICKUP 規則だけ） | I1/I2 と同じ | LIVE `common-furnace-transfer-20261023.json`（21 成功のみ） | 部分的 |
| C7 | 旧 native の画面操作（rename_item、select_trade、set_beacon_effects） | 1.16.1 native | slot / WindowProperty | なし | **見えない**（`Result<()>`、`16/client.rs:4890,4927,4942`） | dispatch のみと明記 | なし | open（低） |
| A1 | Body の通常 submit で Accepted の後に訂正 | 両 | BlockChange / PositionLook / SetHeldItem / WindowItems | `correction()`（`src/body/headless.rs:1147-1259`） | 1.16.1 は `action_observation` / `monitor_changes_after` で**適切**。**1.21.11 は `action_outcome` が gap・world 変更・timeout・取得エラーを黙って `Accepted` にし（`src/body/headless.rs:245-256`）、`Unknown` を捨てる（:247）**。`refresh_trusted` も gap・world 変更を黙ってスキップする（:300-330） | ns-08 の監視を 1.21.11 に移植。それまでは監視の失効を Unknown として返す | 1.16.1 `16/client/body.rs:616,669,758,800,841`。21 は unit test なし。LIVE `body-send-trust-live-20261025.json`（21 は正例のみ） | 1.16.1 **ns-08 で修正済み**、**1.21.11 は open（高）** |
| A2 | Body `ActivateBlock` で画面が開かない・無視される | 両 | C4 と同じ | 期待 block なし（`src/body/headless.rs:783`）。Pose 訂正しか効かない | **別の意味（文書化済み）**: Accepted | C4 の `NoScreenObserved` と結び付ける | なし | open |
| A3 | entity interact / attack | 両 | 拒否のプロトコルはない | なし | **見えない（設計どおり）**: `DispatchReceipt` だけ（`src/client/operations.rs:10-19`） | 許容。上位は entity・体力の観測で判断する | LIVE `entity-interact-at-20261023.json` | 設計どおり |
| A4 | 騎乗の拒否・降車後に再騎乗させられる | 両 | SetPassengers | 騎乗は dispatch のみ。降車は `src/client/vehicle/dismount.rs:105-119` | 騎乗の拒否は最新の乗り物観測でしか分からない。降車の拒否は永久に `Submitted`（:35-36）。`Completed` の後に再騎乗させられても反映されない（:310） | 期限で Unknown。乗り物の変更ログ | LIVE `common-owned-dismount-20261006.json`（成功のみ） | open（低） |

## ns-01〜08 で直った部分の整理

| 項目 | 直ったこと | 残り |
|---|---|---|
| ns-01 | 1.16.1 の爆発によるブロック消去を `BlocksChanged` と爆発ログに出す | P10（16 PlayerControl がノックバックを捨てる） |
| ns-02 | 1.16.1 の dig ACK 訂正を `BlocksChanged` と操作結果に出す。Pending 配置にも届く（B4） | B9 の `DigRecord`/`Survival::dig` に判定がない。B10（1.21.11） |
| ns-03 | chunk/light のデコード失敗で観測を無効化し通知する | 本調査の範囲外 |
| ns-05 | 訂正による接触モデルの破棄を Degraded と `control_changes_after` で返す（P5） | P2（訂正そのものは Running + カウンタのまま） |
| ns-07 | BotManager 中継の Lagged 欠落 | 範囲外 |
| ns-08 | 1.16.1 Body の当時の結果・訂正・監視完全性を分離（P4、B7、A1） | **1.21.11 は Unsupported で、既存経路が失効を Accepted に化かす（A1）** |
| #279 / #286 | foreign close 後の crafting resync、hydration と外部変更の区別（I10、C5 の一部） | C1、C5 の型付き原因 |

## 優先度の提案

1. **B1/B2/B14 厳密配置の拒否で永久 Pending と移動ロック**。1.21.11 ではタイムアウトで接続ごと terminal になる。証拠はすでに受信しているので、分類を追加するだけで済む見込み。
2. **A1 1.21.11 Body の監視失効が Accepted に化ける**。ns-08 の 1.21.11 版。最低限、gap・world・timeout を Unknown にする。
3. **C1 クラフト取り出しの拒否**。`craft_once` が Ok を返すため、上位がクラフト成功と誤認しうる。
4. **P2 PlayerControl のラバーバンドが Running**。owner 単位の訂正ログと、連続訂正時の方針。
5. **I2/I3/I6/I7 インベントリの無効果**。「効果なし」を External・Timeout・永久 Pending と区別する分類。
6. それ以外（P6、P7、P9、C3、C4、B10、B11 など）は、上位が必要とする用途に合わせて順次対応する。

共通方針: 新しい「拒否・無効果」段階は、`accepted=false` のようなプロトコルの bool ではなく、「処理済み境界（1.21.11 の ACK、全量 resync、静穏窓）＋受信値が `before` と等しい」から導く。当時の結果は書き換えない（ns-08 と同じく、当時の結果と後発の訂正を別の軸で返す）。

## 検証の不足

- 実サーバーでの「無効果拒否」を扱うテストと LIVE は両版とも見つからなかった。対象は配置の拒否、クリックの無効果、クラフト取り出しの拒否、画面が開かない場合、1.21.11 の mining ACK と元の値の再送。
- 推測としたもの: vanilla の挙動（レシピ配置・open の拒否で何も送らない、ACK の前に block update が来る）、B1 のロックが再接続まで解けない、I6/I7 の詰まり、P10 の impulse 喪失、B12 の過剰報告。
