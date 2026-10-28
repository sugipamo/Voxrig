# 有限motionの中断原因と制御状態の変更ログ

有限survival motion（`start_predicted_path`）は、受信した位置訂正・自分への速度・爆発・
world変更などで中断すると`MotionRecord.status = RequiresInspection`になり、
最初の理由を自由文`problem`に残す。PlayerControlのownerは、開始後に地形・予測失敗・
mode変化・world変更・期限切れで`Paused`／`Stopped`になる。どちらも従来は
`motion_record()`／`control_record()`を再照会し、自由文を読むしかなかった
（監査 P3・P11、旧09。`docs/design/server-rollback-visibility.md`）。

次の2つを追加した。停止・中断の方針、自由文、既存の再照会結果は変えない。
追加の停止条件や再送もない。

## `MotionRecord.interruption`

`Option<MotionInterruption>`。最初に保持した`problem`が受信した状態変化による
中断だったときだけ、同じ境界で一度だけ設定する。後の問題で上書きしない。
送信失敗・地形・期限切れ・接続終了などのローカルな失敗、中断されていないrunではNone。

- `receive_sequence`: 中断させたpacketの受信ordinal。1.21.11で投影状態の差分から
  判定した種類（mode、姿勢、属性、効果など）は、判定時の最新適用packet。
- `kind: MotionInterruptionKind`（`#[non_exhaustive]`、JSONは`type`タグ）:
  - `Correction { position, rotation }`: 受信した自分の位置（相対flag解決後）。
    teleportと移動拒否はプロトコル上区別できない。原因ではなく、上書きした受信poseを示す。
  - `Velocity { velocity }`: 受信した自分の速度。1.21.11で投影速度が消された場合はNone。
  - `Explosion`、`VehicleChanged`（乗車・passengers・vehicle move）、`WorldChanged`
    （respawn・dimension・reconfiguration）、`GameModeChanged`、`FlightChanged`、
    `PostureChanged`、`AttributesChanged`（1.21.11）、`EffectChanged`、`Died`（1.21.11）。

1.16.1は各受信処理の中断呼出し（位置・速度・爆発・respawn・mode・効果・姿勢・乗車）を
種類へ写す。1.21.11は`stable_context`が拒否するのと同じ優先順
（world、pose、mode、flight、impulse／vehicle、velocity、姿勢、属性、効果、死亡）で判定する。

```rust,no_run
use voxrig::client::prelude::*;
use voxrig::client::survival::MotionInterruptionKind;
# async fn inspect(client: &Client) -> Result<()> {
if let Some(record) = client.survival().motion_record().await? {
    if let Some(interruption) = &record.interruption {
        if let MotionInterruptionKind::Correction { position, .. } = &interruption.kind {
            println!("replan from {position:?} at packet {}", interruption.receive_sequence);
        }
    }
}
# Ok(())
# }
```

## `control_state_changes_after(cursor)`

`Client::control_state_changes_after`、またはPlayerControlの同名メソッド。両版で使える。
ownerの状態遷移を1件ずつ残す接続単位の専用ログ。adapterは状態を確定させた
executorのlock（1.16.1は`common_control`、1.21.11は`State`）を保持したまま公開する。
対象は開始、tickごとの判定、明示停止、quiesce。読取時にも同じ比較を行うが、
確定時にすでに公開済みなら何も追加しない。

`ControlStateChange { sequence, transition }`の`transition: ControlTransition`は次を持つ。

- `session`: version、connection、ownerのworld generation。world変更による停止でも旧world。
- `session_id`: owner。後のownerで付け替えない。`control_kind`: admitした制御モデル。
- `receive_sequence`: 遷移確定時の最新適用packet。受信による停止では原因packet以後。
- `model_tick`・`dispatched_ticks`: 遷移時点のローカルtick数と完全に書いたtick数（受理ではない）。
- `event: ControlStateEvent`（`#[non_exhaustive]`、JSONは`state`タグ）:
  - `Started`: admitされrunningになった。finite handoffで最初からStoppedのmodelは記録しない。
  - `Paused { reason }`・`Resumed`: 予測できないtickで一時停止し、再び進んだ。
  - `Stopped { cause, reason }`: `reason`は`ControlStatus::Stopped`の原文。
  - `Expired { reason }`: 期限切れによる停止（best-effort pathの証拠期限など）。recordは`Stopped`。

`ControlStopCause`（`#[non_exhaustive]`）は、adapterが記録した停止理由から決める:
`Requested`（stop・quiesce）、`WorldChanged`、`GameModeChanged`、`FlightChanged`、
`Vehicle`、`Explosion`、`UnmodeledMotion`、`Correction`、`ModelChanged`、`Died`、
`Completed`（path送信完了・着地・glide終了）、`WriteFailed`、`Disconnected`、
`Prediction`（`movement_refusal`付きの予測失敗）、`Replaced`、`Other`。
1.21.11の"vehicle or explosion motion outside the engine"は、受信した中断packetが
爆発なら`Explosion`、乗り物なら`Vehicle`に絞る。判定できない理由は`Other`で、推測しない。
`reason`は常に残る。状態を確定せずにownerが置き換わった場合は、旧ownerに
`Stopped { cause: Replaced }`を1件残す（通常は起きない）。

cursorと欠落は[劣化ログ](control-degradation.md)・訂正ログと同じ。初回cursorは0、
戻り値`ControlStateLog.sequence`を次回cursorにする。各`sequence`はpacket ordinalや
他のcontrolログのcursorとは別軸。256件（`MAX_RETAINED_CONTROL_CHANGES`）を保持し、
owner交代・world変更・切断でリセットしない。古いcursorは`ErrorKind::State`
（diagnosticを`ControlChangeGap`へdowncastできる）、未来cursorは`InvalidInput`。
読取はログを変えず、切断後も読める。

```rust,no_run
use voxrig::client::control::{ControlStateEvent, ControlStopCause};
use voxrig::client::prelude::*;
# async fn watch(client: &Client) -> Result<()> {
let control = client.player_control(GameMode::Survival);
let cursor = control.control_state_changes_after(0).await?.sequence;
let started = control.start_control().await?;
let log = control.control_state_changes_after(cursor).await?;
for change in log.changes.iter().filter(|c| c.transition.session_id == started.session_id) {
    if let ControlStateEvent::Stopped { cause: ControlStopCause::WorldChanged, .. } =
        change.transition.event
    {
        println!("owner {} ended with its world", started.session_id);
    }
}
# Ok(())
# }
```

## 情報所有

1. 乖離していた複製は、上位が開始時に受け取った`MotionRecord`／`ControlRecord`と、
   接続が保持する現在のrun／owner。正は接続側の現在値と最初の停止・中断原因。
2. 中断原因の正は受信境界そのもの。1.16.1は受信処理の中、1.21.11は受信直後の
   `finite_context_received`・終了処理で`problem`と同時に固定する。
   制御状態の正はexecutorが確定させた`ControlStatus`で、同じlockの中で記録する。
3. 古いRunning前提は遷移の記録時点で破棄する。ログは256件で有界に破棄し、欠落はエラー。
4. 記録はO(1)、読取は最大256件。最新のrecordだけでは途中のpause／resumeや、
   最初の中断がどの受信だったかを復元できない。

## 範囲外

- 停止・一時停止・中断の方針の変更、待機API、共通`EventKind`への通知の追加。
- 位置訂正ごとのログ（#291 `control_corrections_after`）と1.16.1の爆発による停止経路（#298）。

## 検証

単体テストで遷移の順序、置換・handoff・期限切れ、停止理由の分類、256件境界と未来cursorを確認する。
両版のpacket fixtureで、有限motion中の位置訂正・速度・world変更が型付き原因を持ち
地形の失敗が持たないこと、制御中のworld変更と明示停止のログを確認する。
公式serverは`run_motion_control_stop_live.py --version <1.16.1|1.21.11> --accept-eula
--binary <body_probe> --jars <公式jarのディレクトリ> --output <保存先> --evidence <gz>`で再現する。
有限motion中のRCON teleportで`Correction`と受信pose、制御中のNetherへのRCON teleportで
同じownerの`Started`→`Stopped{world_changed}`を確認する。結果は
`docs/evidence/motion-control-stop-live-<version>-20261028.json.gz`。
