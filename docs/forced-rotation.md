# サーバーによる強制回転 (Java 1.21.11 `PLAYER_ROTATION`)

Java 1.21.11 のサーバーは、位置を変えずに自プレイヤーの向きだけを変えるとき
clientbound `PLAYER_ROTATION`（0x47、`ClientboundPlayerRotationPacket`）を送る。
公式 `rotate` コマンドや、位置を含まない `forceSetRotation` が使う。
Java 1.16.1 にこの packet はない（`FACE_PLAYER` は別の扱いで、ここでは対象外）。

## wire 形式と解決

公式 1.21.11 の codec とクライアントの `handleRotatePlayer` で確認した
（`scripts/ExportPlayerRotations.java`、`data/java_1_21_11/player_rotations.json`）。

| field | 型 |
| --- | --- |
| yaw | f32 |
| relative yaw | bool |
| pitch | f32 |
| relative pitch | bool |

相対の角度はローカルの現在角に加算する。pitch は -90..90 に丸める。
解決後の値が有限でなければ、`POSITION` と同じく reader の失敗になる。
位置と速度は変わらない。native client と同じく、受信直後に look-only の
移動 packet（`LOOK`、on_ground=false、horizontal_collision=false）を返す。

サーバーの `~` はコマンド実行元からの相対で、送られるのはサーバーが持つ
プレイヤー角との差分になる。RCON から `rotate <player> ~10 ~` を実行すると、
実行元の角度 0/0 が基準になり、結果は 10/0 になる。

## 見え方

| 経路 | 内容 |
| --- | --- |
| `player_state()` | `rotation` と `rotation_source = Received` |
| 1.21.11 `OwnMotion.received_rotation` | 解決後の角度、literal の相対フラグ、受信順序、world generation |
| `received_player_snapshot().rotation_correction()` | literal の operand（`CorrectionComponent::Absolute/Relative`） |
| `received_player_snapshot().absolute_rotation()` | 両方の角度が absolute なら設定し、どちらかが相対なら無効化する（`POSITION` と同じ規則） |
| `EventKind::PlayerChanged` | 受信 sequence 付きで通知 |
| Body | `Look`・`Move`・ブロック操作などは `Corrected` と `ServerCorrection::Pose` になる |

この packet は位置を受信しない。`position_basis`・`received_pose`・
`last_submission.superseded_at` は変えず、受信 snapshot の `correction()` も
`POSITION` 専用のまま。entity history の `OwnPositionCorrection` も記録しない。
これは contact と replay を区切る位置訂正の fence であり、向きだけの変更は
contact の原点を変えないため。

Body の `ServerCorrection::Pose` は、`position` に変わっていない現在位置を
その出所のまま入れる（`Submitted` や以前の `Received`）。`rotation` の出所は
この packet の `Received { sequence }` になる。現在位置が分からない場合は
`Unknown` になる。要求した `Look` とまったく同じ角度なら矛盾ではないので、
訂正として扱わない。`Chat`・`Command`・`SelectHotbar`・`UseItem` は、
位置の訂正と同じく対象外。

## 実行中の操作

実行中や一時停止中の `PlayerControl` は、受信した時点で保持している
yaw/pitch を受信値に置き換える。そのため、次の tick の移動 packet は受信した
角度を送り、古い照準で上書きしない。`corrections`（位置訂正の回数）は増えない。
その後に `set_controls` を呼ぶと、利用側が明示した角度に戻る。

例外として、次の操作は受信値を採用しない。いずれも受信値は上の経路で報告される。

- 予定 path を持つ owner（`start_path_best_effort` など）。path が tick ごとの yaw を持つため。
- finite な survival motion。計画した yaw で予測しているため。

## LIVE

`scripts/run_player_rotation_live.py` は、公式 1.21.11 サーバー（SHA-1 で確認）に
RCON `rotate` を送る。確認する内容は次のとおり。

- absolute と相対の wire 形式
- echo の内容
- echo より後のすべての移動 packet が受信した角度を運ぶこと
- サーバー側の `Rotation` NBT が受信した角度のまま変わらないこと
- 受信 snapshot の内容
- Body の `Move` が `Corrected` になること

echo より前に送信中だった移動 packet は古い角度を運ぶことがある。native client でも
同じ競合が起きる。結果は `docs/evidence/player-rotation-live-1.21.11-20261027.json.gz`。
