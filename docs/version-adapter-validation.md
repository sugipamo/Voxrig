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

## 第2段階: 1.21.11の受信観測

`versions::java_1_21_11` を追加し、1.16.1と同じ `Client` APIから版を明示して接続する。
接続はoffline login、compression、configuration、dimension registry、playへの遷移と
keepalive・teleport・chunk batchの必須応答を持つ。1.21.11のfull chunk、section update、
単一block update、unload、負のYを扱う。dimension変更と再configurationはcacheを破棄する。

IDとstate propertiesはminecraft-data **3.114.0**のJava **1.21.11 / protocol 774**に固定。
1166 blockの定義とpacket-ID定数の出典・SHA-256は `data/java_1_21_11/source.json`。
`scripts/generate_java_1_21_11.cjs <minecraft-dataのパス> --check` で生成結果を照合する。
Nodeは生成時だけの道具であり、新クライアントの実行時には使わない。

観測には接続ID、world revision、適用済みの受信sequence、ローカル経過時間を付ける。
1.16.1の受信sequenceは未提供なので `None`。接続IDはプロセス内で一意であり、
別プロセスの記録をその整数だけで照合しない。
一つのstate lock内の領域snapshotで、未ロードを空気へ変換しない。
不正なstate packetを処理した後や切断後は、新しいsnapshotの取得を拒否する。

診断用のraw packet記録はpayload合計で最大16MiB / 65536件。
上限超過は `complete=false` とし、それ以後の一部だけを完全な連続記録として残さない。
通常のuse-on-block packetも追加し、読み込み済み・到達距離の確認後に送信する。
送信成功は操作のサーバー受理を意味しない。

**実装していないもの:** 1.21.11の移動物理、inventory、汎用の配置・撤去、
ピストンのBlock Actionから起こすローカル移動・隣接更新、完全なblock entity観測。
online-mode、resource pack要求、experimental feature set等は未対応として拒否する。
既存の1.16.1 `Bot` の機能が全て新版でも使えるという意味ではない。

## 隔離したJava 1.21.11サーバーと階段の診断

公式サーバー、MOD無し、loopback:25577、offline-mode、新規world `isolated`。
9セルの初期観測で石・空気・階段を取得し、`north,bottom,inner_left,false` の全propertiesを確認。
通常のレバー使用でON/OFF、本体の伸長/収縮、移動先の石と丸石階段を観測できた。

一方、隣接するクォーツ階段は `inner_left` のまま、サーバーでは `straight` になった。
全42 packetにそのセルへのstate updateは無く、ピストンのBlock Actionが2件ある。
再接続でfull chunkを取得すると `straight` だった。
[保存証拠・切り分け・次の前提](piston-client-update-prerequisite.md)を参照。
この不一致を、実機一致の成功例に分類しない。

両版のサーバーは正常終了済み。所有するforce-loadは全て解除した。
ライブ試験の再configuration・dimension遷移・長時間稼働・描画クライアントとの比較は未実施。
通常の再接続を、任意の回路の一貫した観測やサーバー確認の代替にはしていない。

## 第2段階のコード検証

新しい検査には、palette形式、負のY、unload、dimension reset、configuration registry、
不正packet後の拒否、bounded captureの欠測、TCP mockでの切断・再接続を含む。
保存した実packet列からON/OFF全770セル・revisionを再現する検査は、
同期の不一致を固定した診断であり、ピストン対応完了の意味ではない。

VoxrigとDustRouteの接続、コマンド確認の除去は未着手。

最終確認は以下の通り。Cargoは逐次実行、`-j1`、テストは1 thread。

| 確認 | 結果 |
| --- | --- |
| `cargo test --offline --locked -j1 --all-targets -- --test-threads=1` | unit 74件成功、全exampleをcompile |
| `cargo test --offline --locked -j1 --doc` | 1件成功 |
| `cargo clippy --offline --locked -j1 --all-targets -- -D warnings` | 成功 |
| `cargo fmt --all -- --check` | 成功 |
| `cargo package --offline --locked --list --allow-dirty` | 成功、版別データと再生fixtureを収録。package buildではない |
| 生成スクリプトの `--check` | 固定したデータ・packet IDと一致 |
| `git diff --check` | 成功 |

初回Clippyのenumサイズ・配列走査・test用Arc共有の指摘は修正して再確認した。
ログは `/tmp/voxrig-stage2-{tests-final,doc-final,clippy-final2,fmt-final,package-final,generation-final}.log`。
unitと静的検査の成功は、上記の階段同期や未確認のライブ試験の合格を意味しない。
