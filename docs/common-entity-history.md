# Entityの受信履歴 (#30・#40)

`Client::entity_history_after(cursor, maximum_records)`は、native decoderが適用した時点の
entityの空間情報・metadata・装備・scale・status・animation・lifetimeとown-playerの位置補正を保存して返す。
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

- 接続ごとに最大8192 **record**、保存値の計上byte数で最大8MiBを保持する。
  計上対象はrecordの固定部分と、その所有する文字列・配列・item data・modifierで、allocatorの
  管理領域や未使用の確保容量は含まない。4096件の通知履歴とは独立している。
  1回の読取は1〜1024件。件数またはbyte予算を超えた最古のrecordを捨て、`gap`で失ったordinalを明示する。
- `None`は先頭からの初回読取。既に古い履歴が失われていれば初回でも`gap`を返す。
  `next_cursor`から再開し、`has_more`なら続きを読む。
  明示的に過去を飛ばす場合だけ`latest_cursor`を使う。
- カーソルはversion・connection ID・**record ordinal**を持つ読み取り専用の型。
  packetのreceive sequenceとは別の順序であり、複数entityの削除など1 packetから複数recordを作れる。
  別接続・別版・未来のカーソルは`State`。不正な読取件数は`InvalidInput`。
  `ordinal()`で読取境界を比較できる。初回pageの`latest_cursor`を上限にして、次のread件数を
  残るordinal数以下にすると、連続した受信の途中でも有限の履歴範囲を処理できる。
  途中で保持窓がその上限を越えた場合も`gap`を処理し、元の上限cursorまでの欠落を明示する。
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
自分宛ての速度packet（ノックバックなど）は`OwnVelocity { velocity }`として受信ごとに保存する。
`OwnPositionCorrection`と違い接触・再生のfenceではない。[速度上書き](own-velocity.md)を参照。

`Metadata`／`Equipment`は**その1 packetの差分**を保存する。後からの現在状態をコピーせず、
元のentry順序と重複index／slotを保つ。値の出典はrecordの`receive_sequence`で、受信済みの
値だけを持ち、pose・子供状態・healthの既定値を補わない。metadataのindex・serializerは版固有。
位置serializerは`BlockPosition` / `OptionalBlockPosition`として符号付き座標と明示的な空値を保持する。
`Other(n)`はserializer番号だけで、その型のpayloadを完全保存した意味ではない。

- `complete:true`は共通decoderがpacketを最後まで読み、余分なbyteがなかったことを示す。
  `false`なら解読できた範囲だけを保存する。modern版で未知serializer／slot／itemがある場合も、
  それ以前の値とpacket境界を残す。legacy版はnativeに読めた装備のうち共通slotに対応するentryを
  順に残すため、未知slotより後の対応entryも含みうる。この場合も`complete:false`になる。
  legacy版の既存native decoderがpacketを拒否した場合には、
  適用済みの履歴を作らない。装備の`Unavailable`は既存のitem変換の欠測で、`Empty`と区別する。
- `entries`は`EntityHistoryValue::Retained { value }`、または`RetentionLimit`。
  **1 packetから履歴へコピーするpayloadの所有byte数が64KiBを超える場合は全差分を保存しない**。
  packetのnative ID・元lifetime・受信順序・`complete`は残す。これはdecoderの部分解読や
  ledgerの古いrecordのevictionとは別の欠落で、entryの一部を完全な差分と見せない。
  最新の`entities()`への既存の適用処理は変わらない。
- spawnが分からない場合やown playerでは`entity:None`。numeric IDを新しいlifetimeへ後付けしない。
  受信したspawnに属する差分は、削除・ID再利用・world変更後も元のworld／spawn sequenceを保つ。

`Scale`はmodern版の属性packetを完全に読んだときだけ、実際のscaleのbaseとmodifierを
元の順序で保存する。未受信を1と推定せず、legacy版にはこのrecordを生成しない。
同じ64KiBの履歴予算を適用する。最新状態の個体別4096byte上限とは独立しており、その上限を
超えて最新scaleが無効化された場合でも、履歴側の予算内なら元の属性入力を保存できる。
履歴の値は過去の受信値で、現在のscaleや物理・装備の作用を証明しない。

`applied_after`は、この接続の履歴ledgerを作ってから、SDKが値を保存した時点までの単調時刻。
**socketへの到着時刻、relayの受信時刻、server tick、UTC時刻、利用側のpoll時刻ではない。**
同じpacket内でもrecord ordinalで順序を確定し、別接続の経過時間を直接比較しない。
readの`receive_sequence`は読取時の境界であり、recordの過去のsequenceと区別する。
pageの`captured_after`は同じ履歴ledgerの時計を読み取った時刻で、更新packetが来ない間も進む。
`captured_after - record.applied_after`でSDK内の経過時間を求められる。最新recordの時刻や
利用側のpoll時刻を新しいサンプルの時刻に置き換えない。
利用側の別の時計へ結合する場合、呼出開始時刻からこの経過時間を引けば、SDK読取までの
待機時間も含めた安全側の古さになる。時計の原点やsocket到着時刻が一致するとは主張しない。
順序を保つため最初のreadで対応する二つの時計の基準を固定し、後のrecordにも同じ対応を使う。
利用側の時計の原点より前になる場合は、変換不能として扱いゼロへ丸めない。

保持対象は、既存decoderが対応する受信spawn、既知lifetimeのmotion、status、animation、
removal、world変更、own補正、metadata・装備の解読済み差分、完全に読めたmodern scale。
未対応のpacket payload全体は保存しない。
motion対象のspawnやmountが解決できない場合、空間値やlifetimeを発明しない。
byte予算によりpacket由来の無制限文字列やNBTを履歴へ蓄積しない。
ProjectileTrack、予測・物理、freshness、戦闘判断、Body policyは利用側で定義する。

## metadata・装備・scaleの実接続検証

2026-10-12（日本時間）、未改変の公式1.16.1・1.21.11へ2クライアントを接続し、合計39項目が成功した。
metadata 48差分、装備10差分、modern scale 3差分を、実際の元packetの値・順序・native IDと照合した。
しゃがみ／解除、子供／大人の連続変更、装備のitem数・空欄・NBT／custom_dataの変更を含む。
modern版の部分metadataも、解読できた範囲と`complete:false`を照合した。
7件ずつのpage、削除後の再読取、実respawn、接続失効後の読取で、過去の値・個体寿命・適用時刻が
変わらないことを確認した。両版のobserver／peer trace errorは0。
[検証記録](evidence/common-entity-data-history-20261012.json)にSDK・runner・binary・元JARのhashと結果を保存した。

```sh
cargo build --locked --example climbing_control_probe --features native
python3 scripts/run_entity_data_history_live.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe \
  --jars /path/to/official-jars --compiled-sdk-revision <built-source-commit>
cargo test --locked --lib history --features native
```

byte予算の境界・上限超過・件数／byteによるevictionとID再利用は、単体テストと既存の
両adapter decoderテストで確認する。実接続で大量packetを合成した検証とは区別する。
元JAR・class・packet・NBTのコピーは公開せず、自作の検証コード・解釈済みの結果・hashだけを保存する。

## 設計判断と検証

既存の[event設計](event-stream-design.md)の通知契約を変更せず、#30で必要となった過去サンプルは
独立したbounded ledgerに置く。通知の受信後に現在状態を読むだけでは、速い連続更新の
過去の速度・角度・順序を回復できないため、この履歴APIを追加する。

両adapterの実decoderによる回帰テストで、生き物とfireballの混在、同じpacketの複数field、
読取前の連続velocity更新、1 packetの複数削除、respawn・ID再利用、8192件超の保持欠落と
分割読取、revocation後の読取を検証する。欠測と受信済みのゼロ、異なる接続のカーソルも区別する。


2026-10-09、SHA-1を照合した未改変の公式1.16.1・1.21.11 server.jarへのloopback実接続でも
各版4項目（合計8項目）が成功し、両packet traceのエラーは0だった。生き物とfireballを
出現・移動・削除し、過去の位置・status・削除後のサンプル、実respawn・own補正、
revocation後の履歴読取を確認した。[証跡](evidence/common-entity-history-20261009.json)には
SDK・probe・runner・公式jar・report・元packet traceのidentityと関連recordを保存する。

最初のfixtureは整数X/Zのcommand中心寄せ（3.0の期待に対し実際は3.5）を見落とし、
次のfixtureは診断後のJSON driver終了を呼ばず終了待機で失敗した。fixtureを修正し、
両版を再実行した。この2 runは成功数に含めない。起動した全server/probeは停止・回収した。
RCONはfixtureの操作と独立した状態確認に使い、SDKの過去値や受信時刻へ混ぜていない。

```sh
cargo build --locked --example migration_api_probe
python3 scripts/run_entity_history.py --accept-eula \
  --binary target/debug/examples/migration_api_probe --jars /path/to/official-jars
cargo test --locked --lib entity_history --features native
```

元packetの一度だけの適用と版ごとのentity更新順は、
[明示的なモデルschedule](common-entity-schedule.md)を参照。

## Internal contact admission checkpoint

Fresh movement owners use a connection-local eviction-boundary checkpoint plus the
retained suffix, captured under the admission receipt lock. The checkpoint is a
bounded, ordered bootstrap reducer program: proven no-op receipts and metadata
fields unused by supported contact dispatch are omitted; stateful instructions,
original lifetimes, provenance, refusals, and map membership mutations keep their
original order. Admission and suffix replay use the same contact reducer. This is
receipt-derived state, never an existing owner's ticked simulation or a latest
spawn snapshot. The declared origin is the captured tail and model tick starts at
zero. Ground, held flight and boat scopes still qualify at admission.

The checkpoint has a separate 8192-instruction / 8 MiB accounted-byte budget.
Exhaustion explicitly refuses admission with `contact bootstrap checkpoint budget
exhausted`; it never silently skips state. A world change supersedes the old-world
checkpoint. This program representation can exhaust its budget under sustained
stateful traffic; numerical reducer-state compaction is not implemented.
Public history remains bounded and reports every eviction gap. Existing owners
continue using their original cursors, and a lagging owner is irreversibly
invalidated on a gap. The internal checkpoint does not repair public history.
