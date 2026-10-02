# develop統合の検証記録

2026-10-02（JST）。利用側のソース公開を要件にせず、Voxrigのclient機能を集約した。
公開APIの設計は[公開client API](public-client-api.md)、変更名と利用側の確認項目は
[移行表](client-api-migration.md)を参照する。

## 取り込んだソース

上流基準は`6434b2cd8d7328d397b34b0151660a9882d844fc`。

| 採用先 | Voxrig上の取込元commit | 主な変更 |
| --- | --- | --- |
| deepplanning | `e290fec63eee9c4da02537c4c80a26a79379d595` | 接続actor、世代と操作context、coherent observation、操作precondition、geometry・inventory・採掘条件の取得 |
| minetool | `d07dd47bfe4eac8fb36c33e826c650665eb55e0e` | イベント過多時の待機、閉じたwindowの遅延slot更新、遮蔽物・未ロードを考慮した作物の可視性 |
| DustRoute | `784c12dba126f5829cd7a8db8542360cc48434c9` | 版別実装の分離、Java 1.21.11、ピストン等の再構成、creative操作、outline照準、記録と観測の共有 |

deepplanningの公開snapshotは元の`a014cd098d47cf35fcc78524072332a8c96f6d7d`と同じtree。
取込後はpackage/crateを`voxrig`へ戻し、特定のplannerやBodyに依存しない名称へ整理した。
かまどと製作のクリック列は`WindowClickSequence`に統一し、利用側が渡すcache予測を
`WindowPrediction`として明示した。クリック列は最初の送信前に構造を検証する。

1.16.1の追加機能は`versions::java_1_16_1`へ配置し、rootの既存APIを同版へ固定する。
1.21.11の型・状態ID・保証は独立した版別APIを維持する。

## client側の検証

検証対象の実装commitは`5da8f92a507226cc4ddb10cf01aac2f7413e2190`。
その後はサンプルの整形とこの検証記録の追加のみを行った。
以下はローカルのclient・mock・fixture検証であり、利用環境の採用検証とは区別する。

| 確認 | 結果 |
| --- | --- |
| 全targetのテスト、Rust 1.97.1、`--locked --all-targets -j1 -- --test-threads=1` | 276件成功、1件スキップ。全exampleをコンパイル |
| docテスト、Rust 1.97.1 | 1件成功 |
| fmt | 成功 |
| 全target Clippy、Rust 1.97.1、`-D warnings` | 成功 |
| 全target Clippy、Rust 1.99.0、`-D warnings` | 成功 |
| rustdoc、Rust 1.97.1、`-D warnings` | 成功 |
| packageの作成と展開後のコンパイル、Rust 1.97.1 | 成功。package一覧だけの確認ではない |
| MSRV、Rust 1.85.0、`--locked --all-targets -j1` | 成功 |

スキップは接続・world変更を伴わない任意の観測materialization性能比較。
native outlineのfixture照合は通常のテストとして実行した。
Clippyの指摘と、1.85で使えないlet chainを修正した。
Atomicの新名称は1.85では使えないため、旧名称の非推奨警告は使用箇所に限定して許容する。

再起動後の検証はCargoを1本ずつ実行し、ビルドを`-j1`、全targetのテストを1スレッドに制限した。
ログはGit管理外の`.local/integration-validation/`へ保存する。

## 利用側の検証とmainへの条件

DustRouteの`8a8852ca6d23c167a23796dbe9b2dec722a397d4`を隔離コピーへ取得し、
`dustroute-mcp`のoptional dependencyだけをこのVoxrigへ向けて検証する。
`voxrig` feature付きのlib・binary・test targetはコンパイル成功。
client連携の試験は以下のfilterを付けて逐次実行した。

```bash
cargo +1.97.1 test -p dustroute-mcp --features voxrig -j1 bridge -- --test-threads=1
cargo +1.97.1 test -p dustroute-mcp --features voxrig -j1 transition -- --test-threads=1
```

bridgeは21件成功、任意の性能比較1件スキップ。transition/記録fixtureは13件成功。
API契約、native propertiesと由来情報、欠測時の拒否、観測共有、操作前の拒否、
記録のtick・順序・打切りを含む。実サーバーの操作・取消・記録は今回の試験では未実施。
検証用ビルドは`CARGO_PROFILE_DEV_DEBUG=0`と`CARGO_PROFILE_TEST_DEBUG=0`でメモリを抑える。

最初の全177件のMCP試験は、client外の回路探索を含むため途中で終了した。
`.local/integration-validation/dustroute.log`にはSIGTERM終了を保持し、全体成功とは扱わない。
上記の対象を絞った成功ログは`dustroute-bridge.log`と`dustroute-recording.log`。

deepplanningとminetoolの利用側は、この環境から取得できる配置先を確認できていない。
公開は不要であり、移行表に従って非公開環境で固定commitを検証できる。
3プロジェクトの同一commitによる採用検証が揃うまでmainへ統合しない。
DustRouteで過去に記録された1.16.1同地点2 Bot移動の不合格も、基準版比較を含めて未解決。

## サバイバル在庫交換の追加統合

2026-10-02（JST）。`codex/survival-construction`の
`da5ad589b47e5ba70e4f579eab5a38f88daf2b75`をdevelopへ追加統合した。
同ブランチは旧DustRoute snapshotから派生していたため、READMEの対応範囲の競合を解消し、
package/crateの`voxrig` 0.2.0、版別構成、既存のclient API整理とRust 1.85対応を維持した。

1.21.11の通常SWAP送信と、両slotの送信後の受信を照合する読出し待機を追加する。
交換の予測をclient cacheへ適用せず、timeout・取消の後もpendingを保持する。
製作・チェスト取得・サバイバルの移動物理や採掘・建築全体の実装は今回の追加に含まない。
1.21.11の`Inventory`に追加されたprivate fieldとfixtureの移行は移行表へ追記した。

| 確認 | 結果 |
| --- | --- |
| 全target、Rust 1.97.1、`-j1 -- --test-threads=1` | 281件成功、任意の性能比較1件スキップ、全exampleをコンパイル |
| docテスト、Rust 1.97.1 | 1件成功 |
| fmt | 成功 |
| 全target Clippy、Rust 1.99.0、`-D warnings` | 成功 |
| rustdoc、Rust 1.97.1、`-D warnings` | 成功 |
| package作成・展開後コンパイル、Rust 1.97.1 | 成功、227ファイル。Java検証ツールとpacket fixtureも収録 |
| MSRV、Rust 1.85.0、全target | 成功 |

追加5件は不正packetの原子性、cursor/window/stackの送信条件、両slotの受信の新しさ、
window変更と接続の所有、実TCPによる部分更新・timeout・重複送信の拒否・待機再開を確認する。
Rust側でnative codec由来のpacket fixtureを照合した。Javaのnative codec検証と
Minecraft実サーバーでのサバイバル操作は、この統合環境では再実行していない。
ログは`.local/integration-validation/survival-merge/`へ保存する。

採用検証の対象はこの追加を含むdevelopの新しい固定commitへ更新する。
旧`ab726cb`の確認結果だけでは新しいcommitの採用検証を完了したとは扱わない。
各利用プロジェクト側で移行・検証し、同一commitの結果が揃ってからmainへ統合する。

## 自身の状態と静止・接地判定の追加統合

2026-10-02（JST）。`codex/survival-construction`の追加commit
`ab55da095a0fa388bd1b10cb87e64f47829427bf`をdevelopへ統合した。
READMEの対応範囲の競合を解消し、版別構成と公開client APIの責務を維持した。
1.16.1の実装には変更を加えていない。

1.21.11の`PlayerState::local_player`で自身の体力・速度・姿勢・属性・effect更新を公開する。
native初期値と受信値の由来を区別し、未受信の体力やeffect一覧の完全性は推定しない。
`standing_context()`は限定的な乾いた静止geometryから通常立位の接地を導出する。
サバイバルの`look()`は送信直前にこの条件を確認し、未対応・欠測では送信前に拒否する。
歩行・落下の物理、時間を要する採掘、サバイバル建築全体の実装は含まない。
公開API仕様と移行表へ`local_player`の追加と`look()`の条件変更を記録した。

| 確認 | 結果 |
| --- | --- |
| 全target、Rust 1.97.1、`-j1 -- --test-threads=1` | 288件成功、任意の性能比較1件スキップ、全exampleをコンパイル |
| docテスト、Rust 1.97.1 | 1件成功 |
| fmt | 成功 |
| 全target Clippy、Rust 1.99.0、`-D warnings` | 成功 |
| rustdoc、Rust 1.97.1、`-D warnings` | 成功 |
| MSRV、Rust 1.85.0、全target | 成功 |

追加7件はnative由来fixtureとの寸法・属性・velocity・接地の照合、
自身とremote entityの分離、不正packetの原子性、reset、geometry変更後の再計算、
欠測・液体・未対応motionの拒否、実TCPでのlook送信と拒否時の無送信を確認する。
Java検証ツールとfixtureのSHA-256が付属manifestと一致することも確認した。
Javaのnative API検証とMinecraft実サーバーでの操作は、この統合環境では再実行していない。
詳細な対応条件は[静止・接地判定](survival-standing-context.md)を参照する。

ログは`.local/integration-validation/standing-merge/`へ保存する。
package検証と隔離コピーのDustRoute互換性確認の結果は統合PRへ記録する。
採用検証の対象はこの追加を含むdevelopの新しい固定commitへ更新する。
旧`44efce7`の結果だけでは今回の候補を検証済みとは扱わず、
各利用プロジェクトの同一commitでの結果が揃ってからmainへ統合する。

## 衝突誤差修正と限定サバイバル操作の追加統合

2026-10-02（JST）。新しいpushを同期し、以下を順にdevelopへ統合した。

| 取込元 | commit | 変更 |
| --- | --- | --- |
| `golemkit/collision-epsilon` | `45dfdad41f41f8ee145e39fa818342a84bd1970d` | 1.16.1の衝突clipで微小な浮動小数点誤差を接触として扱う |
| `codex/survival-construction` | `7915c253ea075926b1576274881e5b426e85c088` | 1.21.11のguard付き送信、loading、採掘・retirement、材料確認付き配置、位置の由来、限定的な歩行・ジャンプ制御と証跡 |

1.16.1の修正は8件のphysics試験で確認してから統合した。
1.21.11側の競合はoperation moduleの説明とthird-party noticesで発生し、
両版の説明・noticeを保持して解消した。package/crateは`voxrig` 0.2.0を維持する。
新コードの3か所のlet chainを同じ条件の`Option::filter`へ置き換え、Rust 1.85対応を維持した。

loading・未解決操作・不確実なframe送信を共通の境界で扱う。
採掘のair受信だけでは元接続の次のmutationを許可せず、明示的な退出確認とfresh recoveryを用いる。
配置は対象block・材料消費・処理sequenceを照合する。survivalのraw `use_on_block`は拒否する。
移動は最大120tickの入力列を利用側が選び、予測と独立接続の同一instanceの新しい位置受信を
区別する。経路探索、汎用地形の物理や完全な建築executorは提供しない。
公開API・移行表・対応一覧へ追加fieldと動作条件を記録した。

| 確認 | 結果 |
| --- | --- |
| 全target、Rust 1.97.1、`-j1 -- --test-threads=1`、MSRV修正後 | 330件成功、5件スキップ、全exampleをコンパイル |
| docテスト、Rust 1.97.1 | 1件成功 |
| fmt | 成功 |
| 全target Clippy、Rust 1.99.0、`-D warnings`、MSRV修正後 | 成功 |
| MSRV、Rust 1.85.0、全target | 成功 |
| rustdoc、Rust 1.97.1、`-D warnings` | 成功 |
| fixture・生成ツール・保存証跡のSHA-256 | 29件の参照がmanifestと一致。motion traceは展開後のhash・sizeも一致 |

スキップは専用Minecraft環境を必要とする4試験と任意の観測性能比較1試験。
Java native oracleとMinecraft実サーバーの試行はこの統合環境で再実行していない。
元ブランチの採掘・退出／再接続・配置の成功記録と、motionの部分成功・壁際停止の失敗記録を
その実行commit付きで保持した。wall-contact試行は全体成功とは扱わず、
後から受信した在庫数の増加も未診断として保持し、最終的な材料の純消費は断定しない。
詳細は[移動制御の実試行](survival-motion-controls.md)と付属の証跡を参照する。

ログは`.local/integration-validation/survival-controls-merge/`へ保存する。
package検証と隔離コピーのDustRoute互換性確認の結果は統合PRへ記録する。
採用検証はこの追加を含むdevelopの新しい固定commitで揃える。
旧`78dfc8c`の結果だけでmainへの統合条件を満たしたとは扱わない。

## 終点clearanceと明示的な再観測の追加統合

2026-10-02（JST）。上記の検証中の再同期でさらに2コミットを取得し、
`codex/survival-construction`の
`30afe28534e8b611ad5da8b77d8c20ecab35b861`までdevelopへ追加統合した。
実装commitは`bc2c13cb6a6927992049cfe0812b248dafcba285`、
後続commitはterminal live trialの保存記録である。

`SurvivalMovementPreview::terminal_clearance`で静止終点の支持と水平1/16blockの
marginを評価する。壁に接したまま停止する計画は最初の送信前に拒否し、
退避をclientが自動で追加しない。完全送信・静止予測を持つ失敗runには、
明示的な`prepare_survival_motion_recheck` / `observe_survival_motion_recheck`を追加した。
元のobserver instanceの新しい位置受信と現在のgeometryを確認する読出しAPIであり、
中断・補正を取り消したり、controlを再送したりするAPIではない。

追加2件は壁接触終点の送信前拒否、明示した退避計画、再観測の新しさ・run／token所有、
残存する形状障害、中断・補正後の拒否、再観測による無送信を確認する。
全targetは332件成功、5件スキップ、全exampleをコンパイルした。
スキップの構成は上記と同じで、実サーバー試行はこの環境で再実行していない。

元ブランチの[終点試行](evidence/survival-terminal-live-20261002.json)では、
歩行・配置、ジャンプ・配置、接触終点の送信前拒否、宣言した壁接触・退避・配置が成功した。
旧試行の失敗を置き換えず、別の実行commitとtraceを保存する。
旧試行の在庫増加は[拾得診断](evidence/survival-motion-pickup-diagnosis-20261002.json)へ分離した。
保存traceのsequence 247の拾得通知と248のslot更新が診断記録と一致することを確認した。
拾ったitemの生成・drop元の証明や、client側のentity物理を追加したとは扱わない。
新しいterminal traceとserver logは圧縮・展開後のSHA-256が保存manifestと一致した。

ログは`.local/integration-validation/survival-terminal-merge/`へ保存し、
残りのRustチェック・package・隔離DustRoute確認の結果は統合PRへ記録する。
最終の採用検証対象はこの追加を含む新しい固定commitへ更新する。
途中の`e4648be`も最終候補の検証済みcommitとして数えない。

## 検査付きclient入口・仮想場面・採掘中断の追加統合

2026-10-02（UTC）。再同期でさらに6件のpushを取得し、確認済みの3件と合わせて9コミット、
`codex/survival-construction`の`1a8f258eeda876b0e3837f8a246b990c3cffb052`まで統合した。

`Client::survival()`と静的なcapability discoveryを追加し、限定サバイバル操作を
`checked_survival`から利用できるようにした。現契約は1.21.11のみで、1.16.1の
従来API・`survival`モジュールを維持する。経路・建築計画・権限・材料予約・永続jobは利用側に残る。
明示的な採掘retirement handleは取消後の読出し再開と一度だけのfresh reconnectをまとめ、
検証済み新sessionの汎用Clientも返す。自動再送・自動reconnectは追加しない。

各tickのheadingを含む`SurvivalControl`と、現在のstateからpreviewを再計算する実行入口を追加した。
完全な限定領域のcaptureと仮想sceneはnative geometry/modelを共有するが、
予測を実操作の受信証拠や許可へ変えない。仮想の照準条件と静止終点の保守的clearanceを分ける。
採掘中の受信inventory変更は最初の原因・sequenceを保持し、手が再び空になっても消さない。
FINISH直前の前提変更もtyped inspectionで返し、transport failureや別の競合を隠さない。
公開設計・移行表に入口とstruct field変更を記録した。

README、API・architecture、crate・operationの説明、採掘の競合を解消した。
既存のRust 1.85対応を保持し、追加されたFINISH前のlet chainも同じ条件の入れ子へ変更した。

| 確認 | 結果 |
| --- | --- |
| 全target / 全example、Rust 1.97.1、`-j1 -- --test-threads=1` | 342件成功、6件スキップ |
| docテスト、Rust 1.97.1 | 4件成功（うちcompile-failの型境界2件） |
| fmt / 全target Clippy、Rust 1.99.0、`-D warnings` | 成功 |
| MSRV 1.85.0、全target | 成功 |
| 追加edge trace / server log | 5ファイルのSHA-256がmanifestと一致、展開可能 |

スキップは専用Minecraft環境を必要とする5試験と任意の性能比較1試験。
実Minecraft・Java oracleはこの統合環境で再実行していない。
元ブランチのedge試行では初期位置x=0.5で候補がなく送信しなかった記録と、
x=0.6へfixtureを変更した後の移動・side配置・退避の成功を別々に保持する。
成功は`f53aef9`の版固有APIでの試行であり、後から追加した共通入口のlive検証へ読み替えない。
材料1個の消費と最終領域の記録は限定edge操作の証跡で、完全な建築・片付けの成功ではない。

ログは`.local/integration-validation/survival-client-merge/`へ保存する。
rustdoc・固定commitのpackage・隔離DustRoute互換性・GitHub CIの結果は統合PRへ記録する。
利用側の移行・実採用検証は今回を含む新しい固定commitで揃える。
旧`467ca6c`の確認だけでmainへの統合条件を満たしたとは扱わない。
