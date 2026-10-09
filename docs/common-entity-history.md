# Entityの受信履歴 (#30)

`Client::entity_history_after(cursor, maximum_records)`は、native decoderが適用した時点の
entityの空間情報・status・animation・lifetimeとown-playerの位置補正を保存して返す。
1.16.1・1.21.11で同じ契約を使う。`events_after`の通知や、後から読んだ`entities()`で
過去の値を組み立てない。`entities()`は現在状態、こちらは過去の受信サンプルである。

```rust,no_run
# async fn sample(client: &voxrig::Client) -> voxrig::Result<()> {
let mut cursor = None;
loop {
    let page = client.entity_history_after(cursor, 256).await?;
    if let Some(gap) = page.gap {
        // gap.dropped_through以前の履歴は欠落。連続したtrackとして扱わない。
        println!("history gap: {gap:?}");
    }
    for record in &page.records {
        println!("{:?}", record.kind);
    }
    cursor = Some(page.next_cursor);
    if !page.has_more { break; }
}
# Ok(())
# }
```

## 読取・保持の契約

- 接続ごとに直近8192 **record**を保持する。4096件の通知履歴とは独立している。
  1回の読取は1〜1024件。保持数を超えた最古のrecordを捨て、`gap`で失ったordinalを明示する。
- `None`は先頭からの初回読取。既に古い履歴が失われていれば初回でも`gap`を返す。
  `next_cursor`から再開し、`has_more`なら続きを読む。
  明示的に過去を飛ばす場合だけ`latest_cursor`を使う。
- カーソルはversion・connection ID・**record ordinal**を持つ読み取り専用の型。
  packetのreceive sequenceとは別の順序であり、複数entityの削除など1 packetから複数recordを作れる。
  別接続・別版・未来のカーソルは`State`。不正な読取件数は`InvalidInput`。
- 各recordは適用時のversion、connection、world generation、packet receive sequence、record ordinalを持つ。
  remote entityのidentityは元のspawn sequenceを含む。IDが再利用されても同じlifetimeに結合しない。
- respawn・dimension変更・再設定は`WorldChanged`で古いlifetimeを無効にする。
  古い履歴は新しいgenerationへ付け替えず保持する。world resetを個別のdespawn受信と偽らない。
- 接続終了・revocation後も残った履歴を読める。読み取りはパケットを送らず、live操作権限を与えない。
  古いentity identityを操作に渡しても既存の接続・generation・spawn検証を通過しない。

## 保存する値と時刻

`Spawn`／`Motion`はそのpacket適用直後の`EntityMotionObservation`を凍結する。
position、body rotation、head yaw、velocity、ground、modern correctionは各フィールドの
`ValueSource::Received { sequence }`を保つ。今回のpacketが更新しなかった値は、以前の
sequenceを保つ。後からのsnapshotやモデル値、未受信の速度・向きのゼロ埋めは使わない。
相対補正のbaselineが解決できなければposition等は欠測のままである。

`Status`／`Animation`／`Removed`は元のnumeric IDとnative codeを持ち、spawnを受信済みなら
そのlifetimeも持つ。spawn不明ならidentityは`None`。own playerはremote spawnとは別に
`OwnPositionCorrection`として保存し、解決済みposeと、解決できたnative velocityを返す。
legacyのown補正にはvelocityがないため`None`である。

`applied_after`は、この接続の履歴ledgerを作ってから、SDKが値を保存した時点までの単調時刻。
**socketへの到着時刻、relayの受信時刻、server tick、UTC時刻、利用側のpoll時刻ではない。**
同じpacket内でもrecord ordinalで順序を確定し、別接続の経過時間を直接比較しない。
readの`receive_sequence`は読取時の境界であり、recordの過去のsequenceと区別する。

保持対象は、既存decoderが対応する受信spawn、既知lifetimeのmotion、status、animation、
removal、world変更、own補正。metadata、装備、未対応のpacket payload全体は保存しない。
motion対象のspawnやmountが解決できない場合、空間値やlifetimeを発明しない。
フィールドを必要以上に広げず、8192件の保持でpacket由来の無制限文字列やNBTを蓄積しない。
ProjectileTrack、予測・物理、freshness、戦闘判断、Body policyは利用側で定義する。

## 設計判断と検証

既存の[event設計](event-stream-design.md)の通知契約を変更せず、#30で必要となった過去サンプルは
独立したbounded ledgerに置く。通知の受信後に現在状態を読むだけでは、速い連続更新の
過去の速度・角度・順序を回復できないため、この履歴APIを追加する。

両adapterの実decoderによる回帰テストで、生き物とfireballの混在、同じpacketの複数field、
読取前の連続velocity更新、1 packetの複数削除、respawn・ID再利用、8192件超の保持欠落と
分割読取、revocation後の読取を検証する。欠測と受信済みのゼロ、異なる接続のカーソルも区別する。
