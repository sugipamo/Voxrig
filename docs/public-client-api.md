# Voxrigの公開client API

Client共通化ブランチでは`Client::survival()` / `Client::creative()`を両版の共通入口とします。
共通型・受信/予測を区別したcapture・版別registryと移行変更は
[Client共通化](client-unification.md)を参照してください。
共通の`Survival::target_block`は限定dry standingから最初のstatic outlineとcaptureを読出します。
queryを採掘/設置の実行許可にしません。[狙い判定の範囲・検証](common-survival-targeting.md)を参照してください。
共通の`Survival::place_cube`と`placement_record`はdefault cubeの一度の設置と、対象・材料の実受信を両版で保持します。
native ACKの有無と取消後の未解決状態は[共通設置の契約](common-survival-placement.md)を参照してください。
下記の追加検査契約は
`Client::checked_survival()`または`Client::survival().checked()?`で明示的に選ぶ1.21.11用の拡張です。


2026-10-03。deepplanning、minetool、DustRouteの改良をVoxrigへ集約する際の設計正本。
各プロジェクトのロードマップや過去のcheckpointは実装経緯の記録であり、公開APIの仕様ではない。

## 責務

Voxrigは接続、プロトコル、状態cache、クライアント物理、観測、指定された低レベル操作を担当する。
目標、資源の意味付け、経路の選択、製作するrecipe、ゲーム内の成功判定、利用側の状態revisionは
呼出側が所有する。名称と型は特定のBody、planner、MCP serviceへ依存させない。

package名は`voxrig`、Rust crate名も`voxrig`とする。公開APIを再設計するため次のreleaseは
`0.2.0`。旧Voxrig 0.1の`Bot`/`BotManager`と通常のimportを保ちつつ、追加fieldを持つstructの
直接構築など、互換性のない変更は移行表に明記する。deepplanning派生版の互換性は約束しない。

## APIの層

- `Client`と`ConnectionConfig`はゲーム版を接続時に明示する入口。
- `versions::java_1_16_1`と`versions::java_1_21_11`は版別の型・registry・操作・保証。
  1.21.11は1.16.1の全機能を持つとは扱わない。版依存のIDやinventory形式を共通型へ丸めない。
- `Bot`/`BotManager`とrootの互換importは1.16.1へ固定する。
- `Client::survival_capabilities()`と`checked_survival::SurvivalCapabilities::for_version()`は
  接続前にも確認できる静的な対応契約。`Client::checked_survival()`はセッションに結び付いた検査付き操作を返す。
  現在は1.21.11の`ObservedDryCubeV1`と明示的に選ぶ`PredictedDryCubeV1`で、
  1.16.1はI/O前に`Unsupported`を返す。
  対応情報は現在の操作許可ではない。`checked_survival`の型は現在のnative 1.21.11表現を共有し、
  他版で同じ意味を持つとは約束しない。従来の`survival::SurvivalState`は1.16.1用として維持する。
- 1.16.1の`lifecycle`、`observation`、`operation`は汎用controllerのための公開API。
  同じ名前の型が存在しても、1.21.11の接続に同じ保証があるとは推論しない。
- `unstable`は既存のraw protocol操作の境界として維持する。

## 接続・観測・操作

`ConnectionGeneration`はプロセス内の接続識別子、`ConnectionState`は接続actorが決める状態。
再接続前のcontextでは操作できない。`OperationContext`は観測sequenceへ操作を結び付け、
`admit_operation`で実行する観測を有効にする。同じcontextの再admissionはidempotentだが、
操作自体の再送はidempotentではない。利用側が重複実行を防ぐ。

`capture_coherent_observation`は複数領域を同じcapture境界で取得する。
`interest_generation`は呼出側の任意の識別値をそのまま返し、`interest`のセル順を保つ。
未ロード、未知のlight、entity省略、event queue overflowとrequest省略を別々に返す。
`CaptureIdentity`のgeometry/inventory revisionは同じ接続でのみ比較する。

`Operation`は送信する操作、`AcknowledgedOperation`は対応するprotocol応答がある操作、
`CleanupOperation`は切断中にも許可する有限の片付けを表す。
`Dispatched`はtransport write、`Acknowledged`はprotocol応答であり、目的達成ではない。
`DeliveryUnknown`では再送前に新しい観測で確認する。

`WindowClickSequence`は製作、かまど、containerの共通クリック列。
どのslotを使うかと手順を呼出側が決め、clientは各クリック直前のpreconditionを検証する。
最初のwriteより前に全列の構造を検証する。4096クリック、各クリック16予測slotを上限とする。
複数クリックはserver上のatomic transactionではなく、一部送信後の失敗は全体成功にしない。

`WindowPrediction`は呼出側が与えるcache予測であり、serverが送ったslot値ではない。
accepted confirmationの後、cacheが指定prestateと一致するときだけ適用する。
観測されるinventoryはclient cacheであり、クリック予測を含み得る。server上の完了の独立証拠には
しない。名称を`prediction`とし、製作の成功やserver observationと取り違えない。

1.21.11の`swap_player_hotbar`は単純スタックの通常交換を送信する。
`InventorySwap`は接続と受信境界に結び付いた送信記録であり、ackではない。
`wait_inventory_swap`は送信後の両slotの受信値・sequenceを照合し、
`InventorySwapObservation`を返す。local slotの予測更新は行わない。
timeoutや取消ではpendingを維持し、同じ接続で読出し待機を再開できる。
staleなscreen revisionでもserverはクリックを実行し得るため、自動再送しない。
この受信記録も独立したserver確認ではなく、1.16.1のcache予測とは異なる契約である。

1.21.11の`PlayerState::local_player`は自身の体力・速度・姿勢・属性・effect更新を返す。
`ValueBasis`でnative初期値と受信値を区別し、effectの残時間や一覧の完全性は推定しない。
`standing_context()`は同じsession lock内で自身の状態と限定的な静止geometryを照合する。
接地はclientの導出結果でありserverのackではない。サバイバルの`look()`は送信直前に
この判定を再実行し、速度・姿勢・周囲のgeometryが不明または未対応なら送信前に拒否する。
観測を保存して後の操作許可として再利用しない。
対応条件は[静止・接地判定](survival-standing-context.md)に記録する。

1.21.11の`wait_until_ready()`は位置と受信terrainに基づく、そのworldの
`PLAYER_LOADED`通知の完全送信まで待つ。`InteractionLoading`はこの履歴を保持するが、
操作のackではない。送信frameの途中で取消・書込失敗が起きた場合は接続を閉じ、
`UncertainDispatch`と`operation_history()`で不確実な試行を保持する。自動再送しない。

`start_survival_mining`/`finish_survival_mining`/`abort_survival_mining`は、限定的な
素手のdirt・stone採掘の各送信を記録する。`wait_survival_mining`等の読出しは対象blockの
新しい受信を照合する。airやABORTを根拠に元接続の次のmutationを許可しない。
継続には明示的なfresh recoveryが必要。独立観測の経路では別接続による同一UUIDの
新しい削除受信を確認してから`reconnect_survival_mining`を用いる。
単一プロフィールの経路は以下の限定契約で別途選択する。利用側が新しい計画を作る。

検査付き入口では`prepare_mining_retirement`が元接続・独立observer・watchを結び付けた
`MiningRetirement`を返す。`close_source`、`observe`/`wait`、`reconnect`は利用側が明示的に呼ぶ。
取消後も同じhandleで読出しを再開でき、clone間でも一度だけの再接続guardを共有する。
`RecoveredSurvivalClient`の`client`・`operations`・`evidence`は同じ検証済み新接続を表す。
接続を返すことは計画・権限・予約・永続jobを引き継ぐことではない。
採掘中の手・選択・screen・cursor・在庫の不整合は受信時に`MiningRecord::inventory_change`へ
最初の原因とsequenceを保持する。手が後で空になっても消さず、別の既知の競合があれば
`sole_cause`をfalseにする。typed inspectionは診断であり、自動回復やitemの由来の証明ではない。

`place_survival_cube`は受信した単純スタックと支持block・空きcellを確認して送信する。
`wait_survival_placement`等は対象block、1個の材料消費、処理sequenceの受信を照合する。
未解決・競合・timeoutは成功やrollbackへ読み替えず、同じ操作を繰り返さない。
survivalのraw `use_on_block`は拒否し、この確認付き経路を使う。

位置誤差を含む照準は、同じfull cubeの同じ面へreach内で到達する条件と、
視線が連続して通り得るcellを検査する。視線の外側のblockは遮蔽物と扱わず、
通過範囲の非air・未知cell・境界の曖昧さは拒否する。有限個のray sampleだけを
誤差範囲全体の証明にしない。立位の支持・身体の接触判定は別に維持する。

`preview_survival_motion`は有限の入力列を予測する読出し、`start_survival_motion`は
1～120tickの入力と位置送信を所有する有限taskである。経路選択は利用側に残る。
`OwnMotion`で自身の受信と送信を分け、移動後の`StandingPositionBasis::PredictedAndObserved`
はmodelの静止予測と別接続の同一player instanceの新しい位置観測を区別して保持する。
観測の量子化誤差を含めてgeometryを再確認し、元の受信速度を予測値やゼロで上書きしない。
取消・補正・形状変化・結果欠測は履歴を保持して止め、自動でreplayや再接続しない。
元ブランチの壁際停止試行は`RequiresInspection`で失敗した記録を保持する。
追加の`TerminalClearance`で終点の静止・支持と1/16blockの水平marginを検査し、
壁に接したままの終点は送信前に拒否する。clientが退避を勝手に入力列へ追加しない。
完全に送信済みの静止予測を持つ失敗runは、明示的な`prepare_survival_motion_recheck` /
`observe_survival_motion_recheck`で新しい観測と現在のgeometryを再確認できる。
これは読出しだけの再評価で、元の失敗履歴を残す。中断・補正済みrunの復旧、移動や自動再送は行わない。
終点拒否と利用側が宣言した壁接触・退避・配置の成功試行は元の失敗とは別の実行commitで保持する。
途中で向きを変える操作は`SurvivalControl`列を`preview_survival_path` / `start_survival_path`へ渡す。
`start_previewed_survival_motion`は送信intent lock内で現在の世代・状態・予測を再計算し、
現在の基準状態・予測と一致しないpreviewをI/O前に拒否する。保存previewは操作許可にならない。

`capture_survival_scene`は完全な限定領域と立位条件を同じlock内で取得し、
`SurvivalScenario`は受信と同じnative geometry/modelで仮想の移動・編集・照準を予測する。
候補の選択は利用側が行う。仮想previewは別型で実移動へ渡せず、未取得セルをairにしない。
`HypotheticalAimRequirement`は選択した契約で将来必要な終点の根拠を示す条件であり、
現在の`StandingPositionBasis`や実行結果ではない。`validate_survival_scene`も読出しだけで、
各実操作には現在の検査と受信結果が必要となる。
liveと仮想のpreviewはtick 0の`initial_frame`も保持し、移動後の静止と
新接続の初期速度・接地状態を区別する。`after_expected_reconnect`は仮想sceneの
新接続初期化を明示し、`ReceivedAfterReconnect`を将来必要な受信条件として返す。
`HypotheticalReconnectBoundary::validate_received_start`は実際に取得した新sceneの
接続・dimension・足位置・受信由来の立位を比較する。retirementの証明、world内容、
captureの新しさは別途必要で、比較成功を操作許可や永続jobの復旧と扱わない。
対応範囲と検証条件は[採掘](survival-mining.md)、[配置](survival-placement.md)、
[移動制御](survival-motion-controls.md)、[仮想場面](survival-hypothetical-scenes.md)、
[検査付きAPIの責務](survival-api.md)を参照する。

### 単一クライアント向けの明示的な契約

`PredictedDryCubeV1`は`start_predicted_survival_path` /
`start_previewed_predicted_survival_motion`で選ぶ。既存の独立観測付き入口は
`ObservedDryCubeV1`を維持し、observerがないことから自動で予測契約へ切り替えない。
`SurvivalMotionStatus::Predicted`と`StandingPositionBasis::Predicted`は完全送信した
有限入力とmodel終点を表す。serverが受理した位置や停止のackではない。
水平1/16blockの`planning_reserve`はmodel内の配置方針であり、実際の位置誤差の上限ではない。
次の操作では世代・dimension・送信・姿勢・属性・補正・現在の受信geometryを改めて検査し、
中断・補正・未対応geometry等は拒否する。元の自身の位置受信を予測値で上書きしない。
仮想計画の`scenario_with_motion_contract`と`PredictedEndpoint`もこの契約を明示する。
予測終点は独立観測が必要な条件を満たさず、仮想条件の比較は操作許可にならない。

`prepare_mining_profile_recovery`はI/Oなしで`MiningProfileRecovery`を用意する。
利用側が`close_source`と一度だけの`reconnect(config, MiningRecoveryTarget)`を呼ぶ。
直接接続した未改造vanilla 1.21.11とプロフィールの排他的所有に限定し、成功した同一
プロフィールの新loginと新しい基準観測を境界にする。local closeだけでは退出を証明しない。
`MiningRecoveryEvidence::boundary`は独立削除受信と同一プロフィールloginを区別する。
両方式は元の`MiningRecord::recovery_attempt`を共有し、login I/O前にclaimを記録する。
取消・失敗・clone・方式変更で二度目のloginを許可しない。元接続を再開せず、履歴や
永続jobから操作権限を復元しない。`OriginalOrAir`は新しい受信対象の照合条件で、採掘や編集許可ではない。
詳細と元commitでの限定実機記録は[予測移動](survival-predicted-motion.md)と
[同一プロフィール復旧](survival-single-profile-recovery.md)を参照する。
この統合環境では実サーバー試験を再実行しておらず、採用先の固定commit検証が必要となる。

1.16.1の`Event::InventorySlotObserved(Snapshot<SlotUpdate>)`はserverのSet Slot受信を
適用時のinventory revisionと共に保持する。queue内の古いイベントと現在のsnapshotを
同一接続内で比較できる。`SlotUpdated`も引き続き発行する。inventory snapshotにはcache予測が
含まれ得るため、このイベントと現在のcacheを混同しない。液体から壁を登るimpulseは
観測済みの乾いた衝突のない脱出領域を要求し、浅い液体で接地している場合は通常jumpを使う。

装備操作、block properties、衝突geometry、採掘条件、entity寸法、item上限はclientの事実を返す。
資源検索は`BlockQuery`/`query_loaded_blocks`へ、移動用の観測は`MovementSnapshot`、
geometryの取得は`GeometryQuery`へ名称を変更する。経路や採取対象を選ぶAPIではない。

## mainを常設する統合運用

2026-10-03に、常設ブランチを`main`のみとする方針へ変更した。
developへ集約したclient APIと既存PRを今回mainへ統合し、以後の修正は最新の
mainから作る短期の作業ブランチで行う。PRの統合先はmainとし、統合後の作業ブランチは削除する。

変更に応じてRustの全target、doc、fmt、Clippy、警告をエラーにするrustdoc、package、
MSRVとCIを確認し、公開APIの変更は移行表へ記録する。配布と利用側の更新は
固定commitやreleaseを基準にする。採用先のソース公開は不要。

deepplanning・minetool・DustRouteの実環境検証は利用側で引き続き行うが、
3プロジェクトの同一commitの結果をmainへの統合待ち条件とする旧運用は終了した。
未実施・不合格・基準版との差はそのまま記録し、main統合を実採用成功の証明にしない。
Voxrig側のunit/mock/fixture成功も実環境の検証を代替しない。
過去の1.16.1同地点2 Bot移動試験の不合格は、上流baseline比較を含めて引き続き確認対象とする。

## 共通の限定採掘

通常のClient handleは`Survival::start_mining` / `finish_mining` / `abort_mining` /
`mining_record`を両版で提供する。共通側が`MiningId`・record/stage・send・target/protocol・
inventory interruptionの型を所有する。modernの既存native intent/recovery APIは維持する。
同じrecordでもlegacy action応答とmodern interaction ACKの意味は区別する。
受信済み空手のdirt/stoneに限定し、除去観測では元接続の次のmutationを許可しない。
[共通Survivalの採掘](common-survival-mining.md)に条件と検証を記録する。
