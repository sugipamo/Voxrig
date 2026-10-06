# 共通Survivalの採掘

`Client::survival()`の`start_mining`、`finish_mining`、`abort_mining`、`mining_record`は、
1.16.1と1.21.11で同じ公開型を使う。通常の利用側にversion分岐は不要。
既存のmodern専用`start_survival_mining`等も維持する。

```rust,no_run
use voxrig::client::prelude::*;
# async fn mine(client: &Client) -> Result<()> {
let survival = client.survival();
// 利用側が対象と編集権限を選択・確認する。対象がなければ送信しない。
let Some(hit) = survival.target_block(4.5).await?.hit else { return Ok(()); };
let started = survival.start_mining(hit.position, hit.face).await?;
// estimated_wait_msはローカルの目安。実際の経過tickや受理の証明ではない。
tokio::time::sleep(std::time::Duration::from_millis(started.estimated_wait_ms)).await;
let attempted = survival.finish_mining(started.id).await?;
let diagnostics = survival.mining_record().await?;
# let _ = (attempted, diagnostics);
# Ok(())
# }
```

共通入口は、健康な通常立位・乾いた既知geometry・接地・通常属性・受信済みsurvival mode、
受信済みの空のselected hand/cursorとplayer screenに限定する。最初のnative outlineが
指定target/faceと一致すること、対象がpropertiesなしのdirtまたはstoneであることを検査する。
足場自身は除去できない。legacyのgeometryは監査済みのair/12種のcubeに限定される。
追加effectの受信がないことは、serverのeffect一覧が完全に空である証明ではない。
工具・他の素材・液体・未ロード・未知形状・複雑なitem dataは今後の実装対象である。

## 送信と観測

`MiningId`は元接続・world generation・attemptに結び付いたopaqueなIDで、Deserializeを持たない。
保存したJSONから操作能力は作れない。`MiningRecord.initial`はSTARTのI/O前に同じadapter境界で
取得したplayer/inventory captureで、後の`player_state()`で置換しない。
`MiningSend.after_sequence`は各commandのI/O前の実受信境界、`dispatched`は完全なframe送信を示す。
START・FINISH・ABORTはそれぞれ一度だけ明示的に送る。自動timer・自動ABORT・再送はない。
FINISH後のABORTもdelayed breakが消えた証明にはならない。

| 記録 | 意味 |
| --- | --- |
| `Mining` | STARTを保持/試行。FINISHも結果もまだない |
| `PendingAfterFinish` | FINISHを保持/試行。取消やABORTでも未解決 |
| `ObservedRemoved` | 元の対象でSTARTより新しい実packetのairを照合。除去したactorの帰属は不明 |
| `RequiresInspection` | 最初のcontext/target/inventory conflictを保持。後の復元やairで消さない |

`target_receipt`はexact targetの実block update。無関係なworld revision、local cacheのair、
ACKだけでは`ObservedRemoved`にならない。結果を保持した後に対象が置き換わっても履歴は変更せず、
現在のworldは`capture`から別に取得する。閉じた接続でも`mining_record`は診断を読み出せる。

legacyの0x1bはaction・packed position・faceを持つ10byte payloadで、modernのinteraction sequenceはない。
`interaction_sequence`は`None`、実際の0x07応答は`LegacyReply`としてaction・success・state・ordinalを残す。
modernはnative global sequenceと、そのsequenceを実際に処理したACKのordinalを`ModernProcessing`として残す。
両者を同じ「成功ACK」とは扱わない。target airもprotocol処理も次のmutationの許可ではない。

## 競合・取消・復旧

保持した採掘がある間は視点変更・選択・移動・二重START等を拒否する。
legacyではconnection actorが通常dispatch/control/clickから採掘を排他し、autonomous physicsも停止する。
FINISH/ABORTのstageをwriter acquisition前に確保し、共通呼出しのwaiterを取消しても
connection-owned taskは同じcommandを再送しない。write失敗は接続/診断に保持する。
modernでは既存のbefore-I/O intent・writer cancellation・不確実なpartial writeのguardを維持する。
writer取得前の取消ではcommandが未送信のまま残り、送信したことや接続が壊れたことを捏造しない。
未送信STARTの後に別のactor等から対象airを受信しても、共通結果の除去成功へは変換しない。

在庫の最初の変更を`inventory_change`へ受信ordinalとslot/selection/cursorの根拠付きで保持する。
後の空手復元でも消さない。別の原因があれば`sole_cause`はfalseになる。
身体のclearanceや足場の一時的な喪失、mode/pose/属性等の変更も各受信境界で保持する。

`continuation_validated`は元接続では常にfalse。air受信・ABORT・ACKで元接続を解放しない。
明示的な同一profileのfresh recoveryは[共通の復旧API](common-mining-recovery.md)を使う。
復旧前のclaimを`recovery_attempt`へ保持し、失敗・取消後も二度目のloginを拒否する。
modern専用の既存recovery履歴/APIを削除・共通履歴へ再解釈しない。

## 検証

同じconsumerを両版のpacket fixtureへ通し、前提のcapture、送信形式、競合拒否、各stageの一度性、
fresh target receiptと履歴/現在worldの分離を確認する。
legacy actorのwaiter取消・write失敗、modern FINISHのwriter取得前取消、空手や足場の一時的変化も検査する。
公式vanilla両版では移動試験とは別の新しい接続でstoneをSTARTし、ローカル待機後にFINISH、
Clientのfresh air受信と別経路のRCONによる対象air・位置不変を照合する。
両版のJVMを同時には起動しない。再実行・実行結果は[共通native検証](common-client-native-validation.md)を参照。
この限定採掘の成功は工具採掘・設置・復旧等の機能parityを意味しない。
