# 自分への速度上書き（ノックバック）

サーバーは自プレイヤーの速度を変えたとき（被弾のノックバック、ダメージ後の
`hurtMarked` など）、他の entity と同じ速度 packet を自分宛てにも送る。
Java 1.16.1 は play `0x46`（`ClientboundSetEntityMotionPacket`、i16/8000 の固定小数）、
Java 1.21.11 は `ENTITY_VELOCITY`（`0x63`、native の packed vector）。

既存の動作は変えていない。1.16.1 は `own_velocity_receipt` に保存して有限 motion を中断し、
1.21.11 は `local_player.velocity` に入れる。実行中の `PlayerControl` は従来どおり
受信速度を body の速度として取り込み、`velocity_updates` を数える。
このページはその受信事実を共通 API で読む方法を説明する。

## 見え方

| 経路 | 内容 |
| --- | --- |
| `received_player_snapshot().velocity_override()` | packet を decode した速度（blocks/tick）。receipt の ordinal はその packet の受信 sequence |
| `entity_history_after()` の `EntityHistoryKind::OwnVelocity { velocity }` | 受信ごとに 1 record。`receive_sequence` は上と同じ |
| `EventKind::PlayerChanged` | 両版で、その受信 sequence 付きで記録する |
| `EventKind::EntityUpdated { native_id: 自分 }` | 1.21.11 だけ。互換のため残す（下記） |

`velocity_override()` は「最後に受信した速度上書き」という受信事実で、現在の速度ではない。
受信後の tick、位置補正（1.21.11 `POSITION` の delta）、爆発の impulse は
この値を変えない。順序は receipt の ordinal で比べる。respawn、再設定、切断などの境界では
他の field と同じく `Invalidated` になる。他の entity 宛ての速度では変わらない。

1.21.11 では、受信と同じ packet の処理中に motion の中断で投影速度
（`local_player.velocity`）が消されることがある。`velocity_override()` は packet を読み直すので、
その場合も受信値を返す。

## 通知の違い

1.21.11 は以前から、自分宛ての速度も `EntityUpdated { native_id: 自分 }` として通知していた。
これは他 entity の更新と同じ意味の通知で、自分の状態の変化とは読めなかった。
互換のためこの通知は残し、同じ受信 sequence に `PlayerChanged` を追加する（順序は
`EntityUpdated` の後）。

1.16.1 の `EntityUpdated` は entity cache にある entity だけで発火する。自分は通常 cache にないので、
自分宛ての速度は以前は何も通知されなかった。今回 `PlayerChanged` だけを追加した。

## entity history と接触モデル

`OwnVelocity` は `OwnPositionCorrection` と違い、接触モデルの fence ではない。
`LivingEntityPacketReplay` は `CorrectionFence` を返さず、`OwnedContacts` も接触 origin を捨てない。
自分の速度は control owner が body の速度として取り込むので、remote entity の再生には影響しない。
接触 policy の packet key は、新しい key を作らずに既存の own-player key
（`OwnPositionCorrection`）を共有する。この key を revoke する policy はなく、callback として
受理する経路もない。そのため frozen baseline（`data/client_api/contact_policy_step1.txt`）は変わらない。

`EntityHistoryKind` は `#[non_exhaustive]` ではない。網羅的に `match` している下流では
新しい variant の arm が必要になる。

## LIVE

`scripts/run_own_velocity_live.py` は SHA-1 を確認した公式 server で、RCON と原 packet だけを使う。
proxy が記録した clientbound の速度 frame を独立した基準にする。

1. 参加直後の無敵時間（60 tick）を待ち、`effect give ... instant_damage` でダメージを与える。
   サーバーは自分宛ての速度（重力分だけの縦成分）を送る。
2. 難易度 easy・夜にして隣に zombie を召喚し、殴られたときの水平ノックバックを受ける。

どちらも、snapshot の値と ordinal、history の record、`PlayerChanged`（1.21.11 は
`EntityUpdated{自分}` も）が frame の decode 結果と一致することを確かめる。

```sh
flock ~/logs/voxrig-live.lock python3 scripts/run_own_velocity_live.py --accept-eula \
  --binary target/debug/examples/body_probe --jars <official-jars> --version 1.16.1 \
  --output <dir>
```

結果は `docs/evidence/own-velocity-live-1.16.1-20261028.json.gz` と
`docs/evidence/own-velocity-live-1.21.11-20261028.json.gz`。
