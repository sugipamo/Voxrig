# 共通のアイテム使用（use_item・release_use_item・use_on_block）

`Client::survival()`と`Client::creative()`の両方に、同じ型で次の3つがある（`Feature::ItemUse`）。

| 操作 | 送るもの | 例 |
| --- | --- | --- |
| `use_item(hand)` | 手に持ったアイテムを対象なしで使う | 食べる・飲む、盾を構える、弓を引く、投げる |
| `release_use_item()` | 使用中のアイテムを離す | 弓を放つ、盾を下ろす、食事を途中でやめる |
| `use_on_block(target, face, cursor, hand)` | 手に持ったアイテムでblockの面を使う | 松明・ドア・レールなどの設置、ドア・レバー・ボタン・チェストの操作 |

```rust,no_run
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
survival.select_hotbar(0).await?;
let dispatch = survival.use_item(Hand::Main).await?;        // 送信の完了だけ
// serverが使用を始めたかは受信した状態で確かめる
let using = client.player_state().await?.using_item;      // Some(ObservedValue { value: Some(Hand::Main), .. })
survival.release_use_item().await?;

// 床のblockの上面に松明を置く。cursorは対象cellの中の当たり位置（各軸0..1）
survival.select_hotbar(3).await?;
survival.use_on_block([10, 63, 20], BlockFace::Up, [0.5, 1.0, 0.5], Hand::Main).await?;
client.wait_for_block([10, 64, 20], std::time::Duration::from_secs(3), |b| {
    b.is_some_and(|b| b.name == "minecraft:torch")
}).await?;
# let _ = (dispatch, using);
# Ok(())
# }
```

## 契約

- 3つとも**送信だけ**の操作で、`DispatchReceipt`は完全な送信を表す。serverの受理や結果ではない。
  結果は受信した状態で確かめる。
  - 使用の開始と終了: `PlayerObservation::using_item`（下記）。
  - 設置・操作の結果: 受信したblockの状態（`wait_for_block`、world capture）。
  - 消費: 受信したinventoryのslot。
  - 開いた画面: `screen_state()`。
- 1.21.11は`DispatchReceipt::interaction_sequence`に、使った操作番号を返す（`use_item`と`use_on_block`）。
  1.16.1のprotocolには操作番号がないので`None`。
- 送信前に、受信したgame modeがhandleのmodeと一致することと、既存の未解決操作（設置・採掘・
  inventory操作・container・乗り物・飛行など）がないことを検査する。自動の再送はしない。
- `use_on_block`は、対象がloaded、world内、目の位置から対象cellの中心まで4.5以内であることを送信前に検査し、
  `cursor`は有限で各軸0..1であることを要求する。視線・遮蔽・面の向きは検査しない。
  手に持ったものとblockの組み合わせで何が起きるか（設置か操作か）はserverが決める。
  - 両版のserverは処理のあとに、対象cellと面の隣のcellのblock更新を送り返す
    （1.21.11はreach外・当たり位置がcellの外の場合を除く）。
  - しゃがみ状態は今のまま使う。しゃがんでチェストに設置したい場合などは、利用側で先にしゃがむ。
- `use_item`は今の向きを使う。1.21.11はpacketに向きを含み、serverはplayerをその向きにそろえる。
- `release_use_item`は公式clientと同じく、位置0・面DOWN（1.21.11は操作番号0）の
  RELEASE_USE_ITEMを送る。使用中でなくても送れる（serverは何もしない）。
- 1.21.11の版固有APIの`use_on_block`は、survivalでは従来どおり拒否する（確認付きの`place_survival_cube`を使う）。
  共通APIの`Survival::use_on_block`は送信だけの別の契約で、材料の消費や設置を確認しない。
  1個のfull cubeを置いて対象と材料の受信まで確認したい場合は、`Survival::place_cube`を使う。

## 使用中の受信状態（`using_item`）

`PlayerObservation::using_item`は、serverから受信した自分のLivingEntityのflags
（1.16.1はmetadata index 7、1.21.11は8。bit 1が使用中、bit 2がoff hand）。

- `None`: このworldでflagsをまだ受信していない。1.21.11は最初の使用まで送られないことが多い。
  1.16.1はjoin時に送られる。
- `Some(ObservedValue { value: Some(hand), .. })`: serverが`hand`のアイテムを使用中と報告した。
- `Some(ObservedValue { value: None, .. })`: serverが使用していないと報告した。
- 送った`use_item`をここに先回りして書くことはしない（受信だけ）。respawnやworldの変化で`None`に戻る。

## 継続操作との関係

継続操作（`start_control`）の実行中も`use_item`・`release_use_item`・`use_on_block`を送れる。
アイテム使用中の減速は[共有物理エンジン](physics-engine.md)が公式clientと同じ規則で再現する
（[比較基準](movement-oracle.md)の場面`use_*`で両版ともbit単位で一致）。

- 入力の縮小: 1.16.1は入力を×0.2（しゃがみの後）。1.21.11は×0.98の後に、使用中のアイテムの
  `minecraft:use_effects`の`speed_multiplier`を掛ける（既定0.2。槍は1.0）。
- ダッシュ: 使用中は開始しない（1.21.11は`can_sprint`がfalseのとき。槍は開始できる）。
  すでにダッシュ中なら、使用を始めても止まらない（公式clientと同じ）。
- 使用中かどうかは**受信した**`using_item`に従う。公式clientは使用を自分で始めた時点から減速するが、
  Voxrigはserverの報告を受け取った時点から減速する（往復の遅れの分だけ、最初の数tickは減速しない）。
  - `release_use_item`を送ると、その時点で減速をやめる（公式clientと同じ）。古い「使用中」の受信は使わない。
  - 使用中のアイテムが受信したinventoryから分からない場合（1.21.11で`use_effects`が決まらない）は、
    sessionを`Paused`にして何も送らない。
- `ControlFrame::using_item`に、そのtickに適用した減速（`ItemUse`）が入る。

## 実サーバーでの確認（2026-10-08）

公式`server.jar`（1.16.1・1.21.11）をoffline-mode・peacefulでlocalhostに起動し、
`examples/item_use_probe.rs`で確認した（consoleで場所とアイテムを用意する）。
どちらの版も次がすべて期待どおりで、serverのlogに警告はなかった。

| 確認 | 1.16.1 | 1.21.11 |
| --- | --- | --- |
| 金のリンゴを食べる（main hand） | 使用中の受信 65 ms後、終了と個数2→1の受信 1.61 s後 | 21 ms後、1.60 s後 |
| 盾（off hand）を構えて下ろす | 使用中→離して44 ms後に非使用 | 同じ（43 ms） |
| 弓を1.2 s引いて放つ | 矢4→3 | 矢4→3 |
| 盾を構えたまま継続操作でダッシュキーを押して歩く→下ろす→歩きながら構える | 減速・ダッシュなし→ダッシュ開始→ダッシュ継続のまま減速。serverの補正0回、serverの最後の位置が予測とbit単位で一致 | 同じ |
| 床の上面へ松明 | 松明が設置され4→3 | 同じ（操作番号4） |
| 素手でレバー・ドア | `powered=true`・`open=true`を受信 | 同じ |
| 4.5より遠い対象 | 送信前に拒否 | 送信前に拒否 |

この確認で、1.21.11の効果packetのflagsにblend（8）があることが分かった。以前は受信時に拒否して接続を
止めていた（金のリンゴの効果で発生）。公式のpacket定義に合わせて受け入れるように直した。


## クリック位置の到達距離

`use_on_block` は、受け取った support cell と `cursor` の和を実際の hit point として、
立った eye からの距離を送信前に検査します。ブロック中心が範囲外でも、指定した面の
クリック位置が 4.5 block 以内なら送信できます。近い面でも、指定したクリック位置が
範囲外なら送信しません。従来の保守的な 4.5 の上限と、ロード・mode・所有・未解決操作の
検査は維持します。`placement_check` の面への最短距離は候補の事前確認であり、
任意の cursor の送信許可ではありません。採掘の中心を使う従来の検査は別の契約として維持します。

Golemkit #152 の保存された半加算器の立ち位置・支持・West 面を平行移動した隔離 fixture は
`scripts/run_block_hit_reach.py` で両公式サーバーに接続して確認します。
新しい世界での fixture 検証と、元の半加算器の全施工・回路出力は区別します。
