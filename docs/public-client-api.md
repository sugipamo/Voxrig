# Voxrigの公開client API

2026-10-02。deepplanning、minetool、DustRouteの改良をVoxrigへ集約する際の設計正本。
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
継続が必要なら、明示的なretirement・別接続による同一UUIDの新しい削除受信・
新接続の基準観測を確認する`reconnect_survival_mining`を用いる。利用側が新しい計画を作る。

`place_survival_cube`は受信した単純スタックと支持block・空きcellを確認して送信する。
`wait_survival_placement`等は対象block、1個の材料消費、処理sequenceの受信を照合する。
未解決・競合・timeoutは成功やrollbackへ読み替えず、同じ操作を繰り返さない。
survivalのraw `use_on_block`は拒否し、この確認付き経路を使う。

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
対応範囲と検証条件は[採掘](survival-mining.md)、[配置](survival-placement.md)、
[移動制御](survival-motion-controls.md)を参照する。

装備操作、block properties、衝突geometry、採掘条件、entity寸法、item上限はclientの事実を返す。
資源検索は`BlockQuery`/`query_loaded_blocks`へ、移動用の観測は`MovementSnapshot`、
geometryの取得は`GeometryQuery`へ名称を変更する。経路や採取対象を選ぶAPIではない。

## 統合順序とmainへの条件

1. deepplanning snapshotのclient機能を整理し、名称・公開契約をVoxrigへ戻す。
2. minetoolのイベント待機・在庫同期・作物の可視性の3修正を取り込む。
3. DustRouteの版分離・1.21.11対応を取り込み、1.16.1機能との共存を検証する。
4. Rustの全target、doc、fmt、Clippy、警告をエラーにするrustdoc、package、MSRVを確認する。
5. developの固定commitでdeepplanning・minetool・DustRouteを各非公開環境で検証する。
6. 3プロジェクトの採用検証が揃ったcommitだけをmainへ統合する。

利用側の公開は不要。Voxrig側のunit/mock/fixture検証は各利用環境の検証を代替しない。
DustRouteに記録された1.16.1の同地点2 Bot移動試験は上流baseline比較を含めて再確認する。
