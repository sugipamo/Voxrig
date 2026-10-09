# 共通移行APIの検証 (#28・#29)

契約は[block raycast](common-blocks.md)と[モード別の基本操作](common-player-control.md)を参照。

2026-10-09、SHA-1を照合した未改変の公式1.16.1・1.21.11 server.jarを、使い捨ての
flat world・offline player・loopbackのみのMinecraft/RCONで起動した。
`examples/migration_api_probe.rs`はfeature `native`を使わず、両版で同じcommon APIを呼ぶ。
各版21項目、合計42項目が成功し、両方のoriginal packet traceにエラーはなかった。

- 40ブロック先のstoneは距離39.5でHit、46ブロック先のbottom slabは上半分を通ればMiss、
  下半分を通れば距離45.5でHitとなった。
- 48ブロックにtarget offsetを加えた実距離のrayと、64ブロックまでのrayが成功した。
  64.001は`InvalidInput`。32ブロックを超えた先の実際の未ロード境界でUnloadedとなった。
- Survival/Creative/Adventure/Spectatorのlookについて、送信receipt・local rotationの
  `Submitted`と元packetの角度を照合した。実受信poseのrotationは上書きされなかった。
  後のRCON読取でもserverのRotationが一致した。これらは同期的なserver ACKではない。
- hotbar選択はSurvival/Creative/Adventureでslot 8の元packetと後のserver読取を確認した。
  Spectatorは`Unsupported`となり、held-slot packetを送らなかった。各modeで不正pitch/slotを拒否した。
- 受信modeの変更後、以前のexpected modeでの入力は`State`となり、要求した角度やslotを送らなかった。
- 実際に死亡・respawnさせ、同じ接続上のraycast結果が新しいworld generationに属することを確認した。

RCONはfixtureの準備・後の独立した読取に使い、SDKのcaptureやdispatch receiptへ混ぜていない。
視界・interaction reach・server-authoritativeな衝突の証明は提供しない。Golemkitを変更せず、
保留中のアプリケーション耐久試験#5にも結論を加えていない。

両TCP adapterの回帰テストでは、状態ロック待機中のmode変更、mode未受信、own poseやstanding
geometryの欠測、未解決dispatch、入力範囲とpacket不送信を検証する。
modernのheld-slot送信をwriter待機中に取り消すテストは、pendingの保持と後続mutationの拒否、
未送信slotが`Submitted`として出ないことを確認する。
raycastは極大・極小のfinite directionでも正規化を保ち、未ロードセルをairにしない。
高さ読取とcaptureの間にworldを切り替える順序を固定したテストは両adapterで結果を拒否する。

[証跡](evidence/common-migration-api-20261009.json)には、SDK source tree・probe/runnerのhash、
公式serverのidentity、report/traceのhashと、42項目の関連フィールドを保存している。
最初のfixtureは48ブロックのstoneをairへ置換した受信前に64ブロック先の結果を評価し、
古い48ブロックのHitで失敗した。対象セルの実受信を待つようfixtureを修正し、両版を再実行した。
この失敗runは成功数に含めない。fixtureが起動した全server/probeは終了処理で停止・回収した。

Java 21と検証済みの公式jarで再現する:

```sh
cargo build --locked --example migration_api_probe
python3 scripts/run_migration_api.py --accept-eula \
  --binary target/debug/examples/migration_api_probe --jars /path/to/official-jars
cargo test --locked --lib common_player_control --features native
cargo test --locked --lib basic_ --features native
cargo test --locked --lib long_raycast --features native
```
