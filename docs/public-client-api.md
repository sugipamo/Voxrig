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
