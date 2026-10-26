# Body API: 受理に基づく共通契約

`voxrig::body` は既存 `Client` の横に追加する API である。headless を残し、将来の Fabric client-only mod バックエンドにも同じ意図・観測・結果の境界を実装できるようにする。Java 1.21.11 と1.16.1のheadlessで観測と全body actionを提供する。Microsoft 認証、Fabric/IPC、既存 API の移行は含まない。

## 受理と完了

完了は「client が完全に送信し、以下の有限な証拠規則で server に受理された」という意味にする。人間の入力の再現、画面の crosshair とクリックの一致、全ての通常 client 制約への一致を成功条件にしない。`Accepted` は指定した監視期間の契約であり、永続的な server 状態、command の成功、chat の配達、将来の変更がないことの保証ではない。

| 状態 | 意味 |
| --- | --- |
| `Pending` | ローカル受付済み。送信、保持処理、証拠の監視の途中 |
| `Accepted` | 完全送信と操作別の証拠条件が成立し、訂正のない監視期間が終わった |
| `Corrected` | server が要求結果を上書きした。`correction` に元の訂正値・出所、`snapshot` に訂正を観測した後の現在の一貫した状態を返す（rollback notice） |
| `Rejected` | ローカルの明示的な拒否・入力不正等。操作の成功を意味しない |
| `Unknown` | ACK 不足、送信の不確実性、切断、world 変更、履歴欠落、期限切れ等により受理を決定できない |
| `Cancelled` | 取消を要求し、未完了の操作を終端にした。既送信の効果は巻き戻せない |

従来の `submit` 経路の状態は一度終端になると変わらない。遅い server 更新は次の観測に現れる（後述の追加 `send_and_trust` 経路では遅延訂正 view も有効にする）。`Unknown` を成功にも再送許可にも読み替えない。サーバーが全操作について個別の成功 receipt を送る、という前提を置かない（#70、#73、#61 のような待ち続ける経路を作らない）。

### ブロック操作

- 1.21.11 の設置・破壊は既存 headless の送信 ledger、guarded writer、単調 interaction sequence を使う。
- 最後の完全送信 sequence を覆う server の block-change ACK（最高 ACK >= 自分の sequence）が必要。ACK は処理済みの証拠であり、単独では要求結果の証明ではない。
- 完全送信と ACK の両方を観測した後、`settle_window` 中に対象結果と矛盾する block update がなければ `Accepted`。材料の slot receipt や不要な own-pose echo は待たない。
- 対象セルの元の block update が要求結果と違えば、同じ値への更新であっても `Corrected`。元の更新値と receive ordinal を返す。最新 cache だけで判定しないため、訂正直後に期待値へ戻った場合も訂正を失わない。
- ACK が来なければ、block cache が期待通りでも期限に `Unknown`。unload/履歴欠落は成功の証拠にならない。
- `PlaceBlock.pos` はクリックするセル、`target` は監視する結果セル。置換可能セル・隣接セル等を controller が明示する。`expected` があれば完全な native state を比較し、省略時は現在の指定 hand の item 名と結果 block 名を比較する。item と block の名称が異なるもの、特定 property を要求する場合は `expected` を渡す。材料の種類による送信制限は設けない。
- `BreakBlock` は見積り不明でも START を送る。Creative は START のみ。Survival は START の処理 ACK 後、まだ air の受信がなければ一度だけ STOP を送り、サーバー側の進捗と air 更新を待つ。ACK だけでは Accepted にしない。取消・期限切れは元 world に ABORT を試み、期限切れは Unknown。`action_timeout` は採掘時間と quiet window を含む。

### 1.16.1 の設置

`PlaceBlock` は指定の `pos / face / cursor / hand / target / expected` を使用し、既存 native actor の writer へ一回送信する。通常立位・接地・raycast の一致・default passive cube・空 cursor・全 inventory receipt は要求しない。`expected` を明示すれば手や選択 slot の観測も送信条件にしない。protocol の有限値・座標表現、現在 world、connection/admission は検査する。

この版は設置 sequence ACK を持たない。完全送信時の coherent gate 内で evidence cursor を確定し、**送信境界より後に適用された元の対象 block update** が期待状態と一致してから `settle_window` を待つ。一致する旧 cache や送信前の更新だけでは `Accepted` にせず、期限に `Unknown`。対象への矛盾した更新は `Corrected`。unload、world 変更、履歴欠落、切断は `Unknown`。外部要因による同じ結果の更新と設置の因果を protocol から証明することはできない。材料減少の receipt は要求しない。既存 `place_cube` とその material/target receipt の意味は変更しない。

既に開始済みの共通 `start_control` session と 1.16.1 の body 設置は併用できる。body はその移動 session を所有・解除せず、呼出し側が `stop_control` する。body の操作は一つずつ行い、他の inventory・設置 stream は並行させない。既存 finite path や未解決な strict 操作の actor admission は維持する。

### 個別の結果 ACK がない操作

look、held movement、slot selection、chat/command、item use は、完全送信の後 `settle_window` 中に server correction/resync/teleport がなければ `Accepted`。chat の自分への echo や movement/look の位置 echo を要求しない。

位置/rotation の server correction、server の slot 選択変更、full inventory resync を元 packet 適用時に記録する。要求した slot と一致する選択 packet は訂正にしない。通常の item 消費を伝える個別 slot 更新は full resync と区別する。ブロック操作では通常の inventory 変化を設置失敗とみなさない。world の変更は `Unknown` にする。

この PR は inventory click 自体を新 API に追加しない。将来追加する場合も、click に万能の per-action ACK があると仮定せず、完全送信・対応する resync/訂正・期限の契約を使う。

## 意図と入力の自由度

`Action` は明示的な yaw/pitch、`Controls`、block pos、face、正確な block-relative cursor xyz、hand を受け取る。将来の entity action でも explicit entity lifetime/ID を渡す。Tweakeroo の flexible placement と同様、mouse で vanilla の照準から到達できる配置だけに絞らない。設置/破壊には新しい body 専用の内部送信入口を設け、既存の reach/raycast/特定 cube・slab 材料フィルターを呼ばず、server の判定に任せる。有限値、protocol の座標表現、cursor 0..1、connection lifecycle/admission は検査する。

headless の movement は既存の ground/fluid/climbing physics と geometry coverage に依存する。`ground_movement_only` がこの制限を明示する。飛行、滑空、vehicle、任意の entity 操作は初期共通 API の外。将来の real-client backend は自身の client tick を駆動し、Rust physics を二重に走らせない。

## 期限、所有、取消

`submit` は操作をローカル受付し、直ちに backend 固有 `ActionId` を返す。操作 task は backend が所有し、呼出し側 future の drop で消えない。ID は instance に閉じ、他 backend の ID は拒否する。既定は `settle_window=300ms`、`action_timeout=10s`。送信待ち・採掘保持・監視を含む絶対期限を submit 時から数える。全 Pending はこの期限までに終端となる。実行 task と status 読取りの両方が期限を検査する。runtime 自体が停止している間の進行は約束しない。

`Move.ticks=Some(N)` は少なくとも N 個の完全送信 client tick を待って入力解除し、その後監視する。tick は50msの local client tick で、server tick ではない。停止との競合で境界の追加 tick があり得る。`None` は最初の tick の完全送信から監視し `Accepted` となった後も入力を保持する。明示解除・cancel、または submit 時からの絶対期限で lease を解除する。`Accepted` は慣性・落下・物理的静止を意味しない。

同じ `HeadlessBody` の clone は所有を共有する。同時には一つの body action/lease を許し、競合は ID 付き `Rejected` にする。yaw/pitch を movement と同時指定したければ `Controls` に含める。body を同一 Client に複数作成したり、別の Client clone の strict mutation と並行実行しないこと（上記 1.16.1 の既存 continuous control との併用を除く）。既存の未解決な診断操作に関する admission gate を消さない。

`cancel` は Pending を終端にし、既送信を取り消せない。終端 action の cancel でも残っている入力 lease は解除する。`release_control` は body が所有する action を取消し、選択された movement owner/dig の解除完了を有限に待つ。失敗時は `Unknown`。不確実な cleanup では既存 disconnect を有限に試み、以後 body の新規操作を拒否する。正常な release は underlying game connection を切断しない。

保持上限（既定256、最大4096）を越えた古い terminal action ID は失効する。`events_after` は Minecraft 受信 stream と別のローカル状態遷移ログで、instance 付き cursor、取得最大数、明示的 `gap` を返す。gap 時は再観測し、履歴の完全性を仮定しない。block/訂正の内部 evidence も4096件に制限し、欠落時は `Unknown` にする。無条件の再送や exactly-once server effect は提供しない。

## 送って信じる経路

`Body<HeadlessBody>::send_and_trust(request: impl Into<ActionRequest>) -> Result<ActionDispatch>`
（`HeadlessBody` にも同名・同 signature）は、ACK・300ms の settle を待たず、
最初の送信境界が完全に書き込まれたら返す追加 API である。
`ActionDispatch { id: ActionId, dispatch: SendStatus }` の `SendStatus::Sent` は
送信済みの意味だけを持つ。未完送信・ローカル拒否・送信前の取消・期限は
`SendStatus::NotSent { outcome: ActionStatus }` を返す。不確実な送信は成功としない。
既存の公開 enum に variant は追加せず、`BodyBackend` の実装要件も増やさない。

| action | Sent の境界 | 返却後も続く処理 |
| --- | --- | --- |
| Look / SelectHotbar / Chat / Command / UseItem | 操作の全 packet の完全送信 | 従来の quiet window と訂正監視 |
| PlaceBlock / ActivateBlock | 指定 hit の全 packet の完全送信 | 従来の ACK／結果観測と quiet window |
| BreakBlock | START の完全送信（Creative・Survival 共通） | Survival の処理 ACK→一度の STOP、結果監視、必要時の ABORT |
| Move Some(N) / None | 設定した入力で最初の client tick の全 packet を完全送信 | Some(N) は N tick・入力解除・監視、None は従来の bounded lease |

送信は操作の終端・所有権の解放ではない。返却後も同時 action は従来どおり拒否され、
future を drop しても backend の task は消えない。cancel・release_control、submit 起点の
絶対期限、cleanup／disconnect fallback は既存と同じ。K1（既知の server 拒否、公式距離・
座標上限）と K2（connection/world/spawn/screen/slot identity、対象・item 照合）の
既存拒否は同じ送信関数内に残す。自動再送はしない。

新経路の ID は `action_status` と `events_after` の読取り時にも保持済みの元受信証拠を
有限 budget で調べる。Pending 中の訂正に加え、Accepted／Unknown 後の訂正も
`Corrected` として読み出し、遷移ログへ一度だけ追記する。これは既存の
`HeadlessBody::action_outcome` と同じ遅延 rollback notice で、元の受理結果や lease を
再開する意味ではない。イベント読取りは対象 ID をまとめた一度の bounded capture を使う。
action／受信 evidence の保持上限、gap、world 変更後には訂正の完全性を保証できない。
後続の無関係な server 操作との因果を protocol から証明することもできない。

呼出し側が送信を完了扱いして先へ進み、後から訂正に反応する用途ではこの経路を使う。
有限期間の受理を確認してから進む用途では従来の `submit` と `action_status` の
終端待ちを使う。こちらの終端不変性・300ms settle・ACK 条件・イベントは変更しない。
厳密な材料／inventory／採掘時間などの診断には既存 Client の strict receipt API を使う。
Golemkit の既存経路には変更がない。

```rust,no_run
# async fn example(body: &voxrig::body::Body<voxrig::body::HeadlessBody>) -> voxrig::Result<()> {
use voxrig::body::{Action, SendStatus};
let sent = body.send_and_trust(Action::Look { yaw: 37., pitch: 13. }).await?;
match sent.dispatch {
    SendStatus::Sent => { /* 呼出し側は送信完了として先へ進む */ }
    SendStatus::NotSent { outcome } => println!("{outcome:?}"),
}
// 後から同じ ID の Corrected を監視する。
let status = body.action_status(sent.id).await?;
# let _ = status;
# Ok(()) }
```

再現用 runner は `scripts/run_body_send_trust_live.py`。SHA-1 検査付きの両版公式 server、
原本 packet proxy と RCON を使用し、ACK／対象更新を保留した状態でも返ること、
占有セルの訂正・Accepted 後の teleport・期限の ABORT と K1/K2 拒否を検査する。
両版各12ケースの実機結果は [`body-send-trust-live-20261025.json`](../evidence/body-send-trust-live-20261025.json)。

## 観測と後続 backend

`BodyBackend` は `capabilities / observe / submit / action_status / cancel / events_after / release_control` の7メソッド。`Body<B>` は static dispatch の facade で、既存 private VersionAdapter を公開しない。

`BodySnapshot` は backend instance、ゲーム版、connection/world generation、revision（headless では受信 high-water mark。同じ値でも local physics が進むことがある）、own position/rotation/health/selected slot/inventory、要求 block/region を一つの lock 境界で返す。個別 block と region は併用できる。未ロードは `None`、slot の欠測も `None`/`Unavailable` で表し、air や empty を捏造しない。item は名前・個数と optional native data で表し、既存の受信 authority token を新 backend に生成させない。

各値は新しい `ValueSource` を持つ。`Received` は元 field packet、`ReceivedCache` は server block cache と capture revision（そのセル固有の receipt ではない）、`Submitted` は完全送信したローカル値、`Predicted` は headless 数値モデル、`LocalCache` は互換 cache、`ClientState` は将来の実 client の現在値。Accepted になっても Submitted/Predicted を Received に格上げしない。既存の `client::ValueSource` enum は変更しない。

future backend は自身の attach/instance/world lifetime、tick/packet hook、入力 lease、取消・接続喪失、履歴欠落を実装する。Java client の現在値から元 packet receipt を捏造しない。人間操作との共存、bridge transport/schema、起動・認証の所有は Fabric 実装の前に別途検討する。この headless PR にそれらの決定は不要。

## 例

```rust,no_run
use voxrig::{Client, NativeBlockState, body::{Action, Body, BodyConfig, HeadlessBody, ObserveRequest}};
use voxrig::client::{BlockFace, Hand};

async fn build(client: Client) -> voxrig::Result<()> {
    let body = Body::new(HeadlessBody::new(client, BodyConfig::default())?);
    let snapshot = body.observe(ObserveRequest::default()).await?;
    let id = body.submit(voxrig::body::ActionRequest {
        world: Some(snapshot.world),
        action: Action::PlaceBlock {
            pos: [2, 64, 0], face: BlockFace::Up,
            cursor: [0.17, 1.0, 0.83], hand: Hand::Main,
            target: [2, 65, 0], expected: Some(NativeBlockState {
                name: "minecraft:gold_block".into(), properties: Default::default(),
            }),
        },
    }).await?;
    // Controller が Pending を poll し、Accepted / Corrected / Unknown 等を分岐する。
    let status = body.action_status(id).await?;
    println!("{status:?}");
    Ok(())
}
```

既存 `Client::connect`、prelude、native feature と strict receipt API はそのまま診断用として利用できる。この API の受理判定で既存 record を完成扱いに書き換えない。

## 検証

状態機械の unit test で完全送信、ACKと静穏期間、訂正、拒否、期限切れ、不完全送信、取消、終端の不変性を検査する。`examples/body_probe.rs` と `scripts/run_body_live.py` は既存 runner の SHA-1 検査付き公式1.21.11 JAR、原本 packet を転送する proxy、独立 RCON oracle を使う。look/move、非slab材料の exact face/cursor 設置、占有セルの訂正、Survival/Creative破壊、chat/command、teleport、入力解除・取消を再現する。さらに原本ACKだけをproxyで遅延させ、server側で破壊済みでも約10秒でUnknownとなり、遅延ACKで終端が変わらないことを検査する。

```sh
cargo build -j2 --example body_probe
python3 scripts/run_body_live.py --accept-eula \
  --server-jar ~/mc/1.21.11/server.jar \
  --binary target/debug/examples/body_probe --output .local/body/live
```

### 手持ちの期待値を指定した使用

`body.backend().use_item_expect(hand, selected_hotbar, item_id)` は両 native version で
`Result<ActionId>` を返す。公開 `Action` / `ActionKind` enum は拡張せず、既存の
unchecked `UseItem` と capabilities の契約も変更しない。内部では world fence と
coherent state lock を保持し、選択スロットと受信済み item ID が一致してから送信する。
不一致・未知状態なら送信せず `Rejected` になり、
`body.backend().use_item_refusal(id)` で期待値と観測値を取得できる。
`Accepted` は消費成功を意味しない。詳細は [common-item-use.md](../common-item-use.md)。

### 拒否削減 step 3: 明示block操作

`Body::place_cube(pos, face, cursor, hand)`はdefault-only材料・first outline・接地を
送信条件にせず、隣接targetと手のblock名を使う。任意の診断は
`HeadlessBody::cube_placement_diagnostics`の警告として別途取得する。
`ActivateBlock`は両版で明示hitを送る。modernはACK＋bounded quiet、legacyは
完全送信＋bounded quietで、menuのcontents受信をrollbackとは扱わない。
実際のscreenは別途観測する。公式serverの距離条件とstrict receipt APIとの違いは
[明示block操作](../explicit-block-interactions.md)を参照。

### 拒否削減 step 4: 1.16.1 の基本action・移動・採掘

`HeadlessBody` は1.16.1でも Look / SelectHotbar / Chat / Command / UseItem /
Move / BreakBlock を送信する。既存の Bot の送信関数・continuous control を再利用し、
protocol encoding を増設しない。Look等のACKのない操作は完全送信後のbounded quietで
Acceptedとなる。これはチャット配信・コマンド成功・食料消費の証明ではない。
`use_item_expect` の選択・受信item ID照合は同じ受信ロック内に残す。

Moveはsteps 1–2のbest-effort contactとRemoteStateDropped診断を使う。
step 5では両版の未知地形も最大0.1 blockのbounded probe（証拠更新なしで20 tick）にし、
`PlayerControl::walking_diagnostics_for`で警告を保持する。strictの床支持・静止証明は要求しない。
補正はCorrected、証拠不足の長期化は既存absolute deadlineでUnknownとなる。
受信poseへの補正はCorrected、world変更・接続喪失・期限切れはUnknownとなる。
同じ接続の既存strict controlと有限path APIは診断用として残し、意味を変更しない。
未知地形や有限歩行の静止条件の緩和はstep 5、他領域のunresolved gate整理はstep 6の範囲。

BreakBlock の step 4 時点の見積り必須経路は、step 7 で下記のサーバー証拠ベースへ置き換えた。
公式serverの採掘距離・world lifetime・既知の手持ち/選択照合と所有者排他は維持する。
旧worldへのABORTは新worldに送らない。

`scripts/run_legacy_body_live.py` は両版に対して同じHeadlessBodyシナリオを使う。
look・hotbar・chat・command・通常使用と期待item付き使用での食事・近傍skeletonがいる段差歩行・
採掘ACK・block置換・pose補正を原本packet traceとRCONで検査する。


### 拒否削減 step 7: 採掘の見積りと送信を分離

両版の `HeadlessBody` BreakBlock は `dig_estimate` を admission に呼ばない。
未知の hardness/tool/enchantment/effect/接地/水中/装備、初期 MissingSelection・MissingSlot は
それだけでは START を拒否しない。既知の負hardness、公式 reach/座標上限、world/mode、
既知の選択slot・手持ち品種の変更、同じ手を変更し得る未解決在庫操作は拒否を残す。
初期未知を空手やslot 0とは解釈しない。START/STOP は同じadapter lock内で再照合する。

Survival は対象のsuccessful START ACK（1.16.1）またはSTARTを覆うsequence ACK（1.21.11）を
待ち、一度だけSTOPを送る。先に対象airが届けばSTOPは不要。公式serverは早いSTOPを
受けてもdelayed destroyを継続するため、ローカルの採掘時間計算は不要になる。
1.16.1のsuccessful solid STOP ACKは進捗中を表せるため、破壊成功にもrollbackにも数えない。
対象への新しいair block updateまたはsuccessful air break ACKとquiet windowでAccepted。
失敗ACK・対象への固体更新はCorrected。ACKだけ・cacheのairだけではAcceptedにしない。
world喪失・履歴欠落・期限はUnknownとなり、一度のABORTを有限budgetで試みる。
STOP後もair受信まではcleanupと共有dig ownerを保持する。ABORTはrollbackの証明ではなく、
特に1.16.1のdelayed destroyは後から完了し得る。自動START/STOP再送や自動再接続は行わない。

`Survival::dig_estimate` は引き続き任意のタイミング診断として呼べる。その成功・値をBodyの
送信やAcceptedの根拠にはしない。`dig` / `begin_dig` / native strict dig / mining と既存receipt、
時刻・同じ手のnative data・estimate変化の照合、取消契約は変更していない。
公開型・メソッド・enumの追加/変更はない。既存APIの動作変更はBody BreakBlockのみである。

### 7段階を終えた使い分け

| step | Body / 明示best-effort経路 | strict診断APIに残すもの |
| --- | --- | --- |
| 1 (#254) | 未監査の近傍contactでも移動送信 | 厳密contactモデルの適用範囲 |
| 2 (#255) | passenger/history欠落・再進入を診断へ落とし、補正poseから継続 | remote数値履歴・scopeの完全性 |
| 3 (#256) | 指定face/cursorで設置・activation、cube診断は任意 | first outline・立位・default materialの証明 |
| 4 (#257) | 1.16.1 nativeの基本action・Move・Breakを共通Bodyへ接続 | native/receipt固有の診断 |
| 5 (#258) | 未知地形はbounded probe、有限歩行は静止証明なしで送信 | 完全床支持・dry地形・exact-zeroのpreview |
| 6 (#259) | receipt未確定を操作領域に限定、bounded watchと遅延Corrected | 元receiptのPending/RequiresInspectionとidentity |
| 7 | 採掘見積りなしでSTART、処理ACKでSTOP、air受信で完了 | strict digの見積り・同じ手・receipt契約 |

Bodyの完了は有限時間内のserver受理/補正観測で、ローカルモデルの完全再現ではない。
K1の既知のserver拒否条件とK2のconnection/world/spawn/screen/slot identityは維持する。
#252は意図的に閉じた方針どおり、全entity型のcontact proof拡張を再導入していない。

既存Bodyの変更を明確にすると、(1) 見積り拒否から送信へ、(2) ローカル時間待ちから
処理ACKでの一度のSTOPへ、(3) modernのsequence ACK単独からair結果の確認へ、
(4) legacyのbreak ACKのみから新しいair block updateも許す判定へ、
(5) STOP後・Creative START後も結果不明ならABORT cleanupを保持する処理へ変更した。
successful solid STOP ACKとcleanupのABORT ACKをrollback扱いしない。
modernでも既知の手/選択の再照合、hardnessと公式reach/heightの独立検査を実施する。
eye height不明時でも、受信済みrangeと水平距離だけで確定する範囲外は拒否する。

公式両版の実機結果・original packet証拠は
[`dig-estimate-live-20261025.json`](../evidence/dig-estimate-live-20261025.json)。
各14ケースで、実slot未受信、3種類の道具、空中、水中、実際より遅い見積りでの送信、
START ACK欠落/STOP後のdeadline、実際のblock置換、K1維持とstrict互換性を検査した。
再現にはnative featureでbuildした `body_probe` と `scripts/run_dig_estimate_live.py` を使う。
modded stateは公式serverには存在しないため実機では作らず、未知hardnessの判定はunitで検査する。

## Motion executorとの所有権境界

有限strictとBody／best-effortの現在model・実行権はconnectionのmotion executorが所有する。
`MotionRecord`の検査結果は実行権ではない。受信correction／impulseを最優先し、
その後の完全送信状態はSubmitted／Predictedのmodelとして継承する（Receivedへ昇格しない）。
補正・world変更・strict preview検査失敗でrun／world generationを同じsend lock内で失効する。
必要なneutral writeとlegacy actor終了ACKが完了してから次ownerを認め、旧workerの終了は新ownerに作用しない。
world変更後の旧入力解放は送らず、partial writeのK1と欠測・未監査K2の拒否は維持する。
再導出は近傍地形と現在pose／momentumから行い、strict previewだけが最大120tick分の計算を行う。
Golemkitの運用API、既存strictの引数・結果型・成功／observer証明条件は変更しない。
