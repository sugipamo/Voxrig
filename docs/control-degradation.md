# 1.16.1 best-effort接触モデルの劣化通知

`PlayerControl::start_control_best_effort`と`start_path_best_effort`は、接触の
admission・refresh・予測が成立しなければ、remote数値モデルを破棄し地形だけで続ける。
受信した位置訂正でoriginが変わる場合も同じownerを再アンカーする。
この事実は通常の開始・入力更新・停止・再照会結果である
`ControlRecord.degradation: Option<ControlDegradation>`へ載る。
`status`と`movement_refusal`は従来どおり停止状態・停止原因を表す。
劣化は停止・拒否でも、サーバーの移動受理でもない。追加の拒否条件は設けない。
送信と受信訂正を観測する既存のCorrected方針を維持する。

`ControlDegradation`は最初の破棄時に固定され、以下を保持する。

- `session`: version、connection、world generation。
- `session_id`: 当該owner。別ownerや別worldへ付け替えない。
- `receive_sequence`: 破棄時までに適用された最新packetの境界。
  Correctionではoriginを変えた実際の位置packet ordinal。
- `model_tick`: 破棄したローカル数値tick。Admissionは0、Predictionは試行tickを含む。
- `phase`: Admission / Refresh / Prediction / Correction / Terrain。
- `reason`: 元の構造化理由を包む`MovementRefusal::RemoteStateDropped`。

`contact_diagnostic_for(id)`も同じ最初の理由を返す。後の地形失敗や位置訂正で
最初の理由を上書きしない。停止後も保持し、owner交代では新しいownerへ持ち越さない。
`entity_contacts`・contacts・teamのremoteモデルは破棄と同じ境界で消す。
Noneは当該1.16.1 ownerでまだ破棄がないことだけを意味し、接触の永久保証ではない。

## 後発の破棄と欠落

`Client::control_changes_after(cursor)`、またはPlayerControlの同名メソッドを使う。
接続単位の専用ログで、破棄元が直接記録しnative broadcastを中継しない。
記録はモデル破棄と同じ状態境界で行い、Admissionはownerのbind成功後に公開する。
最初の破棄をownerごとに1件保存する。これは全制御状態遷移のログではない。

初回cursorは0。戻り値`ControlChangeLog.sequence`を次回cursorにする。
各`ControlChange.sequence`はpacket ordinalとは別軸。結果はcursor以後の全保持suffixで、
読取はログや他readerを変更せず、game modeを要求しない。
256件（`MAX_RETAINED_CONTROL_CHANGES`）を保持し、owner交代・world変更・切断でも
cursorと履歴をリセットしない。接続cloneは同じログを共有する。

古いcursorは失った件数を含む`ErrorKind::State`、未来cursorは`InvalidInput`。
Stateの`error.diagnostic().downcast_ref::<ControlChangeGap>()`から失った件数`missed`、
保持suffix直前の`resume_after`、当時のtail`sequence`を取得できる。
欠落を空成功に変えない。State時は接触予測への保証を撤回し、最新recordと受信観測から
計画を再構築する。残った履歴は`resume_after`から読む。意図的に過去を捨てる場合も、
利用者がgapの`sequence`を選んで明示的に再ベースラインを確立する。
この選択は失われた履歴を復元しない。
モデル破棄自体は入力を止めないため、上位は継続・明示停止・再計画を選べる。

```rust,no_run
use voxrig::client::prelude::*;
# async fn observe(client: &Client) -> Result<()> {
let control = client.player_control(GameMode::Survival);
let cursor = client.control_changes_after(0).await?.sequence;
let started = control.start_control_best_effort().await?;
if let Some(warning) = &started.degradation {
    // このownerについてremote接触を予測できるという前提を撤回する。
    println!("{}: {:?}", warning.session_id, warning.phase);
}
let changes = client.control_changes_after(cursor).await?;
for change in &changes.changes {
    println!("owner {} lost contacts", change.degradation.session_id);
}
let next_cursor = changes.sequence;
# let _ = next_cursor;
# Ok(())
# }
```

1.21.11は本修正の対象外。共通traitの既定実装によりログは`Unsupported`、
新しいrecordフィールドはNone。従来の版別診断は存続する。
Noneを読む前にversion/supportを区別し、未対応を「劣化がない」と解釈しない。

## 情報所有

1. 乖離していた複製は、Voxrig ownerの実際の接触モデルと、上位の「このownerはremote接触も
   予測する」という前提。サーバーの位置・entity受信値が正であり、数値予測は受理証拠ではない。
2. モデル可用性の正は、そのownerが実際にremote stateを捨てた事実。
   ownerが一度だけ生成したimmutable記録を結果と専用履歴へ渡す。
3. 旧remote stateと上位の接触保証は破棄境界で撤回する。同じowner内では再構築しない。
   新ownerの再入場は明示操作。過去の記録は256件の有界保持で破棄し、欠落はエラーになる。
4. 現ownerの警告参照はO(1)（結果のcloneを除く）。履歴読取は最大256件。
   接触モデルの再構築は選択領域・entity historyの読取と再検証が必要。
   最新状態だけから失われた途中の破棄理由は復元できない。

## 検証

1.16.1 TCP/packet fixtureでadmission、後発spawn、位置訂正、継続・停止結果、
owner交代、256件境界、未来cursor、world変更・切断後の欠落を確認する。
既存のcontact scopeテストでstaged predictionと地形へのfallbackも検証する。
公式1.16.1は`run_contact_best_effort_live.py --version 1.16.1 --accept-eula
--binary <body_probe> --jars <公式jarのディレクトリ> --output <保存先>`で再現する。
元packet traceとRCONの独立位置を使い、通常結果とログの一致、後発spawn、origin訂正後も
同じownerが送信を続けることを確認する。位置訂正はRCON teleportによるもので、
サーバーが接触移動を自発的に拒否したという主張ではない。

2026-10-27の公式1.16.1 LIVEは18項目すべて成功。
[公開証跡（gzip JSON）](evidence/control-degradation-live-1.16.1-20261027.json.gz)に
各結果・通常recordとログの対応・サーバー／バイナリ／ソース／元traceのhashを保存した。

既存のcontact-envelope監査は`control.rs`全体のhashを固定しているため、
同ファイルのfingerprintを更新した。今回の差分は破棄通知と診断の保持のみで、
strict admission・scope・物理計算・fallbackの送信条件は変更していない。
