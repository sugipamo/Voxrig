# PlayerControlの位置訂正ログ

`PlayerControl`の継続操作は、受信した自分の位置（teleport・移動拒否の押し戻し）を
公式clientと同じようにモデルへ適用し、多くの場合`Running`のまま送信を続ける
（best-effort ownerは常に続ける。strict ownerは接触originを失う訂正で従来どおり停止する）。
`ControlRecord.corrections`は適用回数だけを数える。適用は次のtickで最新の受信poseだけを
読むため、tick間に2回訂正されると1回に潰れる。上位は「進んでいる」と
「押し戻され続けている」を区別できなかった（監査 P2、`docs/design/server-rollback-visibility.md`）。

`Client::control_corrections_after(cursor)`、またはPlayerControlの同名メソッドは、
running／paused ownerが受けた位置packetを受信ごとに1件残す。両版で使える。
記録は位置packetの受信処理の中で、tickの適用より前に作る。
受信hookを経ずにtickで初めて適用されたposeも、適用時に1件だけ記録する。
停止・paused・再開の方針は変えない。追加の停止条件や再送もない。
strict ownerを停止させた訂正も、停止前の受信として1件残る。

## 記録の内容

`ControlCorrectionChange { sequence, correction }`の`correction: ControlCorrection`は次を持つ。

- `session`: version、connection、world generation。
- `session_id`: 受信時に選ばれていたowner。owner交代で付け替えない。
- `receive_sequence`: 位置packetの受信ordinal。
- `model_tick`: 受信時点でownerが終えたローカルtick数。
- `submitted`: 受信前に最後に書き終えた位置packet（`model_tick`と足元座標）。
  最初の書き込み前はNone。送信であってserverの受理ではない。
- `model_position`: このpacketが置き換えたローカルモデルの足元。
  tick間に先行する未適用の訂正があれば、その受信座標（公式clientは順に適用する）。
- `received_position`: 相対flagを解決した受信座標。
- `delta`・`distance`: `received_position - model_position`と、そのユークリッド長。
- `received_velocity`: poseと一緒に適用した速度。Noneはモデル速度を変えない。
  1.16.1の位置packetは常に速度0を適用する。

プロトコル上、RCON・plugin等のteleportと移動拒否（moved wrongly／too quickly）は
区別できない。記録は「送信・予測が受信poseで上書きされた」事実と差分であり、原因ではない。
原因を推定する場合も`submitted`と`received_position`の関係から上位が判断する。

## cursorと欠落

接続単位の専用ログで、`control_changes_after`（劣化ログ）と同じ有界ledgerの実装を使う。
cursorと保持は劣化ログと独立している。連続する押し戻しで訂正が多数届いても、
劣化記録を追い出さない。

初回cursorは0。戻り値`ControlCorrectionLog.sequence`を次回cursorにする。
各`sequence`はpacket ordinalとは別軸。結果はcursor以後の全保持suffixで、読取はログを変えない。
256件（`MAX_RETAINED_CONTROL_CHANGES`）を保持し、owner交代・world変更・切断でもリセットしない。
古いcursorは`ErrorKind::State`（diagnosticを`ControlChangeGap`へdowncastできる）、
未来cursorは`InvalidInput`。欠落を空成功に変えない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn observe(client: &Client) -> Result<()> {
let control = client.player_control(GameMode::Survival);
let cursor = control.control_corrections_after(0).await?.sequence;
let started = control.start_control().await?;
let log = control.control_corrections_after(cursor).await?;
let pushed_back = log
    .corrections
    .iter()
    .filter(|c| c.correction.session_id == started.session_id)
    .filter(|c| c.correction.distance > 1.0)
    .count();
println!("{pushed_back} corrections; next cursor {}", log.sequence);
# Ok(())
# }
```

## 情報所有

1. 乖離していた複製は、ownerのローカル送信・予測と、serverが持つ自分の位置。正はserverの受信pose。
2. 訂正の事実の正は位置packetの受信そのもの。受信境界でownerが一度だけ不変の記録を作る。
   `Received.pose`は最新値だけなので、tick側からは途中の訂正を復元できない。
3. 送信・予測はpacketの受信時に置き換わる。記録は256件の有界保持で破棄し、欠落はエラーになる。
4. 記録はO(1)、読取は最大256件。最新のrecordやentity historyからは、
   潰れた途中の訂正や当時の`submitted`を復元できない。

## 範囲外

- 連続訂正時の停止・一時停止（例: N回連続で停止するopt-in）は方針変更になるため含めない。
- 有限survival motionの型付き中断原因（監査 P3）、乗り物の訂正（P6）は別作業。

## 検証

両版の単体・packet fixtureで、tick間の2件の訂正、受信hookなしの適用、停止後の受信、
別handle・切断後の読取、256件境界と未来cursorを確認する。
公式serverは`run_control_correction_live.py --version <1.16.1|1.21.11> --accept-eula
--binary <body_probe> --jars <公式jarのディレクトリ> --output <保存先> --evidence <gz>`で再現する。
strict ownerを停止させるteleport、best-effort owner実行中のRCON連続teleport 2回、
RCONで移動速度attributeを上げた際のvanillaの"moved too quickly"による押し戻しを、
owner・status・送信継続・server logと照合する。
