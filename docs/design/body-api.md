# Body API: 受理に基づく共通契約

`voxrig::body` は既存 `Client` の横に追加する API である。headless を残し、将来の Fabric client-only mod バックエンドにも同じ意図・観測・結果の境界を実装できるようにする。この PR は段階 1〜3（契約、共通 facade、Java 1.21.11 headless）のみ。Microsoft 認証、Fabric/IPC、既存 API の移行は含まない。1.16.1 は capability を空にし、操作・観測は `Unsupported` を返す。

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

状態は一度終端になると変わらない。遅い server 更新は次の観測に現れる。`Unknown` を成功にも再送許可にも読み替えない。サーバーが全操作について個別の成功 receipt を送る、という前提を置かない（#70、#73、#61 のような待ち続ける経路を作らない）。

### ブロック操作

- 設置・破壊は既存 headless の送信 ledger、guarded writer、単調 interaction sequence を使う。
- 最後の完全送信 sequence を覆う server の block-change ACK（最高 ACK >= 自分の sequence）が必要。ACK は処理済みの証拠であり、単独では要求結果の証明ではない。
- 完全送信と ACK の両方を観測した後、`settle_window` 中に対象結果と矛盾する block update がなければ `Accepted`。材料の slot receipt や不要な own-pose echo は待たない。
- 対象セルの元の block update が要求結果と違えば、同じ値への更新であっても `Corrected`。元の更新値と receive ordinal を返す。最新 cache だけで判定しないため、訂正直後に期待値へ戻った場合も訂正を失わない。
- ACK が来なければ、block cache が期待通りでも期限に `Unknown`。unload/履歴欠落は成功の証拠にならない。
- `PlaceBlock.pos` はクリックするセル、`target` は監視する結果セル。置換可能セル・隣接セル等を controller が明示する。`expected` があれば完全な native state を比較し、省略時は現在の指定 hand の item 名と結果 block 名を比較する。item と block の名称が異なるもの、特定 property を要求する場合は `expected` を渡す。材料の種類による送信制限は設けない。
- `BreakBlock` は Creative なら START 一回、Survival なら既存 dig estimate に基づき START→保持→STOP を送信する。instant block は START のみ。取消・期限切れで保持中の dig は元 world に ABORT を試みる。結果は air を期待する。見積りより長い採掘は `action_timeout` を適切に設定する必要がある。

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

同じ `HeadlessBody` の clone は所有を共有する。同時には一つの body action/lease を許し、競合は ID 付き `Rejected` にする。yaw/pitch を movement と同時指定したければ `Controls` に含める。body を同一 Client に複数作成したり、別の Client clone の strict mutation と並行実行しないこと。既存の未解決な診断操作に関する admission gate を消さない。

`cancel` は Pending を終端にし、既送信を取り消せない。終端 action の cancel でも残っている入力 lease は解除する。`release_control` は body が所有する action を取消し、選択された movement owner/dig の解除完了を有限に待つ。失敗時は `Unknown`。不確実な cleanup では既存 disconnect を有限に試み、以後 body の新規操作を拒否する。正常な release は underlying game connection を切断しない。

保持上限（既定256、最大4096）を越えた古い terminal action ID は失効する。`events_after` は Minecraft 受信 stream と別のローカル状態遷移ログで、instance 付き cursor、取得最大数、明示的 `gap` を返す。gap 時は再観測し、履歴の完全性を仮定しない。block/訂正の内部 evidence も4096件に制限し、欠落時は `Unknown` にする。無条件の再送や exactly-once server effect は提供しない。

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
