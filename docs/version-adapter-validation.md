# バージョンアダプタの検証記録

2026-09-29、`codex/dustroute-client`。上流基準は `6434b2c`。

## 第1段階: 1.16.1 の分離

1.16.1 固有のクライアント、registry、衝突・移動、item/window 等の処理を
`versions::java_1_16_1` へ移した。crate root の既存公開 API はこの実装を再公開し、
従来の呼出しを維持する。共通の frame/圧縮処理、エラー、snapshot は版別実装の外にある。
全ての状態モデルを無理に共通化する変更はしておらず、版固有の情報は版別型に残す。

新しい `Client` は `ConnectionConfig` の `MinecraftVersion` に従って接続する。
観測は version、接続ID、world revision、取得時刻、未ロードを明示した全セルを返す。
領域の読み出しは1回の world lock 内で行い、ID をその版の全 properties へ展開する。
これはローカル cache の観測であり、サーバー確認済みとは表示しない。

この段階では 1.21.11 の接続は明示的な `Unsupported` でネットワーク I/O 前に拒否する。
1.21.11 が実装された場合、この段階の拒否テストは過去の状態を示すものになる。

変更前の60 unit tests、doc test、fmt、全target Clippy、package list は成功。
変更後は65 unit tests、全exampleのcompile、doc test、fmt、全target Clippy、package list が成功。
新しい検査は state properties の復号、壊れたregistry、領域上限、版の拒否、
未ロード・切断・再接続の観測区別を対象にしている。

## 隔離した Java 1.16.1 サーバー

公式 server JAR の SHA-1: `a412fd69db1f81db3f511c1463fd304675244077`。
既存の EULA 承諾記録を利用し、新規ワールドを `.local/java-1.16.1` に作成した。
loopback の port 25576、offline-mode、creative、Java 21 で実行した。
ユーザーの既存ワールドや DustRoute の実行環境は変更していない。

成功した確認:

- `version_observation_probe`: 9セルを読み、石・空気・階段の全 properties を保持。
  階段は `facing=north,half=bottom,shape=inner_left,waterlogged=false`。
- `api_surface_probe`: 2 Botで command 20、tag 144、recipe 859、chunk 81、
  共有section 43を観測した。既存API・初期同期・chunk共有の検査が成功。
- `version_control_probe`: 単独Botで初期同期・静止を待ち、2秒の入力により
  約8.375ブロック移動。入力区間で server position correction は0件。
  耐久性や移動tick処理時間の基準を満たしたという主張はしない。
- `version_interaction_probe`: serverから石を支給し、通常のdig packetの応答と
  Airへの変化を確認後、place packetで石へ戻ったことを観測した。

不合格・調整した試行も保持する:

- 階段の最初のfixtureは、隣接階段が無くサーバーでも `straight` だった。
  `inner_left` を期待した最初の試行は不合格。コンソールで実状態を確認し、
  `(0,4,1)` に西向きの階段を置いた別試行で `inner_left` と観測の一致を確認した。
- 既存 `control_probe` は初期同期直後に同じ地点の2 Botへ入力し、
  `server corrected normal control movement` で不合格だった。
  原因の確定や上流バイナリとの比較は未実施。静止後の単独Bot試験をこの不合格の
  取り消しや同等条件の再試験とは扱わない。

試験専用の force-load 9 chunkを解除し、コンソールの `stop` で正常終了した。
試験ワールドとサーバーログは `.local/java-1.16.1` に保持する。
これは1.16.1の範囲の確認であり、1.21.11や階段のピストン移動時の観測一致は未検証。

## 続く作業

1.21.11 の接続・configuration・registry・chunk/block更新を独立したアダプタへ追加する。
VoxrigとDustRouteの接続、コマンド確認の除去はまだ行っていない。
