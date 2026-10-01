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
