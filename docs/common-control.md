# 共通の継続操作（P7）

`Client::survival()`の`start_control`・`set_controls`・`stop_control`・`control_record`は、
キーを押し続ける操作を1.16.1と1.21.11で同じ型で扱う（`voxrig::client::control`）。

```rust,no_run
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
survival.start_control().await?;                  // 現在のSDKモデルと向きで、キーを離した状態で開始
survival.set_controls(Controls { forward: 1, sprint: true, ..Default::default() }).await?;
tokio::time::sleep(std::time::Duration::from_secs(2)).await;
let record = survival.control_record().await?;   // 予測した位置・状態、補正の回数
survival.stop_control().await?;                   // ダッシュとしゃがみを離して終了
# let _ = record;
# Ok(())
# }
```

## 動き

- clientが50 msごと（clientの時計）に[共有物理エンジン](physics-engine.md)を1 tick進め、
  そのtickにclientが送るものを送る。押したキーは`set_controls`で置き換えるまで有効で、次のtickから効く。
  - 1.21.11: 入力パケット（前後左右・ジャンプ・しゃがみ・ダッシュのキー。変化したときだけ）、
    ダッシュの開始・停止コマンド（変化したとき）、位置・向き・接地・横の衝突。
  - 1.16.1: ダッシュの開始・停止、しゃがみの押下・解除（変化したとき）、位置・向き・接地。
  - ダッシュは、キーを押していて公式clientの開始条件を満たすときに始まり、停止条件で止まる（ダブルタップは使わない）。
- 送った位置は**送信であってserverの受理ではない**。`ControlRecord::frame`は予測値。
- `ControlFrame::eye_in_water`は、予測した姿勢での目が水中かどうか（公式の`updateFluidOnEyes`）。
- 受信したものは公式clientと同じように取り込み、続ける。
  - serverからの位置（テレポート・補正）: 位置を置き換える（`corrections`に数える）。
  - 自分への速度（ノックバック等）: 速度を置き換える（`velocity_updates`に数える）。
  - 開始時は現在のSDKモデルとローカルのyaw/pitchを使い、移動・ジャンプ・しゃがみ・ダッシュの入力を離す。
    同じworldでの停止・再開でも、すでにモデルへ取り込んだ古いノックバックを再適用しない。
    停止中に届いた新しい位置・速度は受信順に取り込み、古いノックバックで新しいテレポートを上書きしない。
    開始境界までの受信は初期状態に含み、補正・速度更新のカウンタは開始後の更新を数える。
  - 移動速度の属性（受信した修飾子にclient自身のダッシュ修飾子を加える）、効果、満腹度、飛行許可、ネザー（溶岩の流れ）。
- 予測できないtick（範囲外の地形、未ロードのchunk）では`Paused`になり、そのtickは何も送らない。
  毎tick同じ状態からやり直し、予測できれば`Running`に戻る。
- 1.16.1は停止中も進む互換モデルの現在の位置・速度・接地・落下距離から再開する。
  互換モデルがまだ進んでいなければ共通エンジンの状態全体を引き継ぎ、進んだ後は古い水中・姿勢状態を再利用しない。
- 1.21.11は同じworldの自分の位置受信が必要。位置送信が未解決、速度の受信基準が不明、
  または他の操作による位置送信の後に新しい位置受信がない場合は開始を拒否する。
  最初の再開パケット、繰り返し再開、落下と着地、停止後の位置受信を両版の公式serverで追加検証し、
  元のbytes・SDK revision・binary/JAR hashを[レビュー検証記録](evidence/common-control-restart-review-20261009.json)に保存した。
  `scripts/run_control_restart.py --accept-eula`で再実行できる。
- 次の場合は`Stopped`になる: `stop_control`、切断、world（死亡からの復帰・次元）の変化、game modeの変化、飛行、乗車、
  爆発などエンジンの外の動き、死亡、送信の失敗。止まったsessionは再開しない（`start_control`で新しく始める）。
- 同じworldで接続が使える場合、自動停止でもダッシュ・しゃがみ（1.21.11では全入力）を解除する。
  `stop_control`が対象sessionを取得した後は、待ち手がキャンセルされてもClientのtaskが入力解除と記録保持を完了する。
- sessionの間は、静止を前提にする操作（移動の予測・有限の移動・照準・採掘・設置・収納を開く）は拒否する。
  1.16.1では、sessionの間`Bot`自身の物理と`set_control`のloopを止める（開始時に`Bot`のキーが離されていることを要求する）。
  停止中はSDKの通常の物理loopが進むため、再開はその時点のモデル位置・速度から始まる。
  1.21.11では停止した共通sessionのモデルを引き継ぐ。停止後に別の有限移動・位置送信を行った場合、
  新しい位置受信がない限り、そのsessionの古いモデルを使った再開は拒否する。
  同じ座標に戻っていても、別の移動送信を現在のモデル速度の証拠として扱わない。

## 範囲と制限

- 地形・液体・効果の範囲は[エンジンの説明](physics-engine.md)のとおり。梯子・つる・足場を登れる。
  ジャンプを押すか、梯子・つるの壁へ進むと登り、しゃがむと梯子・つるで滑り落ちるのを止める。
  足場ではジャンプで登り、しゃがむと下へ降りる。梯子と向きが一致する上の開いたトラップドアも登れる。
  泡の柱の上昇・下降、水面からの出入りにも対応する。飛行・乗車は扱わない。
  泡の柱は両版の公式サーバーへ実接続し、上下・水面脱出・停止を
  [元の通信とRCON座標](evidence/common-fluid-control-20261009.json)で確認した。
- 1.16.1の深海探索者・ソウルスピードの靴は、まだ環境に反映していない。
- アイテムの使用中（盾・弓・食事）の減速は、受信した`using_item`に従って再現する。sessionの間も`use_item`・
  `release_use_item`を送れる（[アイテム使用](common-item-use.md)）。
- 時刻はclientの時計で、serverのtickとは同期しない（公式clientと同じ）。

## 停止・再開の検証（2026-10-09）

`run_control_resume.py` が公式 1.16.1・1.21.11 サーバーへ共通 Client で接続し、
歩行中・落下中の停止と再開、照準を保持した解除入力、着地、停止中に届いた新しい位置補正、
停止した記録の保持を確認する。再開後の元の movement packet と RCON 座標を保存する。
[検証記録](evidence/common-control-resume-20261009.json) に実 source・binary/JAR hash、
両版の計11確認と初回失敗を記録した。914テスト、strict Clippy、Rust 1.85 の全ターゲット確認も通過した。

1.16.1 の着地は10 block、1.21.11 は対応範囲内の4 blockの落下を使う。
1.21.11 の最初の10 block落下は、既存の `fall-distance reset sweep` 制限で `Paused` になった。
この試行を着地成功とは扱わず、最終fixtureでは10 block落下が同じ制限で送信を止め、
記録を保持することを別に確認する。物理の範囲外判定は変更していない。

```sh
cargo build --locked --features native --example climbing_control_probe
python3 -B scripts/run_control_resume.py --accept-eula
```

停止中のモデル進行は版ごとに上記の契約に従う。この確認は単発ジャンプ要求、装備交換、
消費側の共通移動API移行、保存worldの元事象再現を完了したことを示さない。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`（1.16.1・1.21.11）をoffline-modeでlocalhostに起動し、`examples/continuous_control_probe.rs`で確認した。
consoleで前方に2 blockの深さの水場を作り、同じ接続で次を続けて行った:
歩く20 tick → ダッシュ10 → ダッシュジャンプ12（水に飛び込む）→ 水底でしゃがんで歩く10 → 止まる10 →
ジャンプしながら泳いで渡り、岸に上がる80 → 止まる20 → 歩いている途中でconsoleからテレポート → 歩いて止まる。

| 版 | 送ったtick | serverの補正 | 最後の位置（予測） | serverの位置（`data get entity`） |
| --- | --- | --- | --- | --- |
| 1.21.11 | 約210 | 1（意図したテレポートのみ） | `[5.5, -60.0, 33.85794463056311]` | `[5.5d, -60.0d, 33.85794463056311d]` |
| 1.16.1 | 約210 | 1（意図したテレポートのみ） | `[232.5, 4.0, -190.35791443778544]` | `[232.5d, 4.0d, -190.35791443778544d]` |

テレポートの後もsessionは`Running`のまま続き、押したキーで歩き続けた。どちらの版も、serverが記録した最後の位置は予測とbit単位で一致した。

## 登りと停止の実接続検証（2026-10-08）

`examples/climbing_control_probe.rs`は共通APIだけを使う。`scripts/run_climbing_control.py`が
localhost限定の公式サーバーを版ごとに起動し、実際のblock更新を進めながら検証する。
RCONの`data get entity ClimbingProbe Pos`でserver側の座標を独立に取得し、
元の通信をそのまま転送するproxyで入力解除パケットを確認する。

両版で次が通過した:

- 高さ65から、梯子・つる・足場をジャンプで32 tick登る。
- 梯子・つるの壁へ20 tick前進して登り、12 tickしゃがんで高さを保つ。
- 足場で12 tickしゃがむと下へ降りる。
- 水浸しの梯子・足場を登り、梯子では壁へ前進して登る。水中の足場はしゃがんで沈む。
- 梯子と向きが一致する開いたトラップドアを通って登る。向きが異なるものと閉じたものは越えない。
- 停止すると入力解除を送り、以後の記録は変化しない。別sessionを開始できる。
- ダッシュ中の外部テレポートを1回だけ取り込み、`Running`のまま継続する。
- creativeへのmode変更では自動停止してダッシュと入力を解除する。
- 切断後も最終の`Stopped`記録を読める。

登り中の予期しない補正は両版とも0回。予測とRCON座標の比較は、独立した時計のずれを考慮して
最大0.4 blockの差まで許す。通信で実際に送った解除と、待ち手のキャンセル後の解除・記録保持を調べる回帰テストも追加した。

再実行（Java 21が必要。`--accept-eula`はMinecraft EULAへの同意）:

```sh
cargo build --locked --example climbing_control_probe
python3 -B scripts/run_climbing_control.py --accept-eula
```

サーバー・Client・パケットのログと`report.json`は`.local/climbing/live/`へ保存する。
水浸しとトラップドアを含む両版25項目ずつの通過結果と元reportのhashは
[検証記録](evidence/climbing-vehicle-control-20261008.json)へ保存した。
既存の公式jarを使う場合は`--jars /absolute/path/to/downloads`、別のCargo targetを使う場合は
`--binary /absolute/path/to/climbing_control_probe`を指定できる。jarのSHA-1を検証する。
