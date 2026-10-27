# 1.16.1 のチャンク／光の受信失敗

Java 1.16.1 の `CHUNK_DATA` (`0x21`) と `UPDATE_LIGHT` (`0x24`) が解釈できなければ、
その接続の観測を撤回して reader を終了する。以前のように native
`Error { kind: "chunk_decode" / "light_decode" }` だけを出して受信を続けない。
サーバーが送った新しい状態を反映できない以上、旧キャッシュを現在値として扱えないためである。
正常な操作の送信条件・サーバー訂正の扱いは変更しない。再送・再接続も自動では行わない。
Java 1.21.11 の実装は変更していない。

## 共通 API での扱い

- 失敗 packet の適用境界で、snapshot を `ReaderFailure` によって失効し、元の
  エラー種別・packet ID・handler 名・payload 長を `snapshot_failure` に保存する。
  `received_player_snapshot()` はこの診断をエラーとして返す。診断は終了後も取得できる。
- 同じ coherent gate 内で共通 `EventKind::ObservationInvalidated` を直接記録する。
  `receive_sequence` は失敗した packet の連番。native broadcast の購読は不要である。
- `chunk`、`loaded_chunks`、`chunk_context` は、その境界から旧キャッシュを返さずエラーにする。
  reader の supervisor が終了処理を完了する前の短い区間も対象になる。
- reader はエラーを伝播し、既存の接続終了処理が `Disconnected` を記録して待機を起こす。
  失敗後に届いた正常な packet は適用しない。接続状態は未確認終了 (`Unknown`) になる。
  native 利用者には既存の終端 `Error { kind: "connection", message }` で診断が届く。
- 既に呼出元へ返した immutable な観測値を後から書き換えることはできない。
  `ObservationInvalidated` または `Disconnected` を受けたら、その接続から導いた計画と
  保持済み観測の現在値としての利用を取り消す。座標すら読めない失敗もあるため、範囲は接続全体。

```rust,no_run
use std::time::Duration;
use voxrig::client::prelude::*;

async fn monitor(client: &Client) -> anyhow::Result<()> {
    let mut cursor = 0;
    loop {
        // State（保持欠落）を含め、Err なら完全な履歴として続行しない。
        let log = client.wait_for_events(cursor, Duration::from_secs(30)).await?;
        for event in log.events {
            match event.kind {
                EventKind::ObservationInvalidated => {
                    // この接続の観測から導いた経路・安全条件をここで捨てる。
                    // エラーには失敗 packet の診断が残る。
                    let diagnostic = client.received_player_snapshot().await.unwrap_err();
                    return Err(diagnostic.into());
                }
                EventKind::Disconnected => anyhow::bail!("connection observations withdrawn"),
                _ => {}
            }
        }
        cursor = log.cursor;
    }
}
```

通知は既存の `events_after` / `wait_for_events` の有界ログ（最新4096件）を使う。
保持範囲より古い cursor は `ErrorKind::State`。欠落を完全な suffix として返さない。
終了後も保持済み通知と診断を読める。最後の `Disconnected` を読み終えた cursor からの
待機は終了エラーを返す。保持欠落後に古いキャッシュを再読しても回復にはならない。
必要なら新しい接続で、サーバーから届く全チャンクと光を受け直す。

`EventKind` は既に `#[non_exhaustive]` なので、新しい variant は additive。
公開シグネチャの変更はない。native のデコード失敗が接続終了になる振る舞いの変更は、
不完全な観測を正常として継続する不具合の修正である。

## 情報の所有

1. **乖離していた複製**: サーバーの最新チャンク／光、Voxrig の旧 world/light cache、
   それを保持した上位の経路・環境条件。デコードできなかった受信分が旧 cache に反映されず、
   共通通知を追う上位も旧状態を現在値として維持できた。
2. **現在の正**: 世界状態の権威はサーバー。クライアントでは、受信 reducer が
   「新しい状態を解釈できず、この接続の観測は継続不能」という事実と診断を一元所有する。
   通知はその境界だけを指し、世界や診断の独立したコピーを持たない。
3. **古い状態の破棄時点**: 失敗確定と同じ coherent gate 内で snapshot の有効性を撤回し、
   world 読み出しをエラー化する。上位は無効化通知・終端通知・履歴欠落を見た時に
   依存計画を破棄する。共有された immutable snapshot は過去の記録としてのみ残る。
4. **再導出のコスト**: 同じ cache の再読は回復不能。再接続と初期 world 受信、対象領域の
   section/light 配列の再構築・走査が必要。時間・通信量は再取得するチャンク数と payload に比例し、
   接続待ちも発生する。失敗した原 payload の内容は最新状態から再構成できない。

## 検証と再現

`world_decode_failure_*` fixture は、既存チャンク・光のある状態での置換失敗、座標を含まない
切断 payload、光の欠落・余剰、supervisor 完了前の読み出し拒否、実 reader の終了と
共通待機への通知、後続 packet の不適用、event cursor の保持欠落を検証する。

LIVE は既存の `run_climbing_control` / `PacketTraceProxy` 手法で、SHA-1 確認済みの
公式 1.16.1 サーバーを隔離起動する。まず無改変のチャンク・光・文脈の受信を確認する。
その後、別々の実接続で chunk/light 各1 packet の末尾1 byte をプロキシだけで落とし、
framing/compression を正しく付け直す。共通無効化→終端の順序、診断、旧観測の読み出しエラーを確認する。
**不正 payload は障害注入であり、公式サーバーが自然に不正 packet を送った証拠ではない。**
保持欠落の保証は fixture の結果であり、LIVE で検証したとは扱わない。

```sh
cargo build --locked --example world_decode_failure_probe
python3 -B scripts/run_world_decode_failure_live.py --accept-eula \
  --binary target/debug/examples/world_decode_failure_probe \
  --jars /path/to/official-jars --output .local/ns03-live
```

証跡: [公式受信＋障害注入](evidence/world-decode-failure-live-1.16.1-20261027.json.gz)。
source SHA-256、公式 jar SHA-1、元 payload、注入後 payload、共通通知、読み出しエラーを保存する。
