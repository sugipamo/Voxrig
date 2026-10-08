# 共通の継続操作（P7）

`Client::survival()`の`start_control`・`set_controls`・`stop_control`・`control_record`は、
キーを押し続ける操作を1.16.1と1.21.11で同じ型で扱う（`voxrig::client::control`）。

```rust,no_run
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
survival.start_control().await?;                  // 受信した最新の位置から、キーを離した状態で開始
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
- 受信したものは公式clientと同じように取り込み、続ける。
  - serverからの位置（テレポート・補正）: 位置を置き換える（`corrections`に数える）。
  - 自分への速度（ノックバック等）: 速度を置き換える（`velocity_updates`に数える）。
  - 移動速度の属性（受信した修飾子にclient自身のダッシュ修飾子を加える）、効果、満腹度、飛行許可、ネザー（溶岩の流れ）。
- 予測できないtick（範囲外の地形、未ロードのchunk）では`Paused`になり、そのtickは何も送らない。
  毎tick同じ状態からやり直し、予測できれば`Running`に戻る。
- 次の場合は`Stopped`になる: `stop_control`、切断、world（死亡からの復帰・次元）の変化、game modeの変化、飛行、乗車、
  爆発などエンジンの外の動き、死亡、アイテムの使用中の受信、送信の失敗。止まったsessionは再開しない（`start_control`で新しく始める）。
- sessionの間は、静止を前提にする操作（移動の予測・有限の移動・照準・採掘・設置・収納を開く）は拒否する。
  1.16.1では、sessionの間`Bot`自身の物理と`set_control`のloopを止める（開始時に`Bot`のキーが離されていることを要求する）。

## 範囲と制限

- 地形・液体・効果の範囲は[エンジンの説明](physics-engine.md)のとおり。梯子・つるの登り、泡の柱、飛行、乗車は扱わない。
- 1.16.1の深海探索者・ソウルスピードの靴は、まだ環境に反映していない。
- アイテムの使用中（盾・弓・食事）の減速は扱わない。sessionの間は`use_item`を拒否し、
  受信した`using_item`が使用中なら開始を拒否し、実行中なら`Stopped`にする（[アイテム使用](common-item-use.md)）。
- 時刻はclientの時計で、serverのtickとは同期しない（公式clientと同じ）。

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
