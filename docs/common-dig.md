# 共通の採掘（`Survival::dig`）

`client.survival().dig(target, face)`は、手に持っているもので任意のblockを掘る。両版で同じ型。
`dig_estimate(target)`は、送らずに掘る時間だけを返す。

```rust,no_run
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
let estimate = survival.dig_estimate([10, 64, 3]).await?;   // tick数・正しい道具か・速さ
let record = survival.dig([10, 64, 3], BlockFace::Up).await?;
println!("{} ticks, removed: {}", estimate.ticks, record.removed);
# Ok(())
# }
```

## 動き

公式clientと同じ順で送る。

1. START_DESTROY_BLOCKを送る。進み方が1以上なら、ここで壊れる（`ticks`が0）。
2. 1 tickごとの進み方を`ticks`回足して1に届くまで待つ（clientの時計。1 tickを足す）。
3. STOP_DESTROY_BLOCKを送る。serverは自分が数えたtickで70%以上進んでいれば壊す。
4. 受信したblockの状態が変わるのを最大1秒待ち、`removed`と`final_state`を返す。
   `removed` は元の world で loaded air を観測したときだけ true。アンロードや別の non-air block は false。
   壊れなかったことはerrorではない（serverが決める）。1.21.11は`sequences`にSTARTとSTOPの操作番号が入る。

70%（0.7）の判定は低 TPS や START/STOP の処理遅延を吸収する保証ではなく、
server が数える tick が不足すれば、この余裕があってもその場で壊れない。

送信前に、受信したgame modeがsurvivalであること、対象がloadedで目から4.5以内であること、
未解決の操作がないことを検査する。START/STOP の直前にも元の world/mode/選択した hand と採掘時間の入力を照合する。
各 adapter は同じ状態ロックを保持したまま検査から完全なパケット送信まで行う。
対象・目の水位・接地・player capture もそのロック内で採取する。
1.16.1 の同じ item/個数の通常摩耗は、非負で単調増加する Damage だけの差を許容する。
他の NBT、item、個数、選択の変更は拒否する。
START では採掘見積もり全体の完全一致を要求する。STOP では、硬さ（`hardness`）と
正しい道具か（`harvestable`）が同じで、`progress_per_tick` が初期値以上、`ticks` が初期値以下なら
速くなった見積もりも許容する。両方の見積もりの `speed` と `progress_per_tick` は有限かつ正でなければならない。
f32 を直接比較し、誤差の許容幅は設けない。遅くなった場合や、これらの条件に合わない変化は拒否する。
着地や水から出た場合も、待機時間は最初の見積もりの `(ticks + 1) * 50 ms` のままで短縮せず、
`DigRecord.estimate` も最初の見積もりを保持する。`ticks == 0` の瞬時採掘は従来どおり STOP を送らない。
この許容はローカルな整合性の判断であり、block が壊れる保証ではない。server は自分の tick と状態で決める。
server の接地（`on_ground`）や水中の状態は受理した移動 packet に由来し、こちらより遅れたり異なったりする。
水位や pose の違いによって server 側の採掘速度がこちらより遅くなることもある。
拒否メッセージは変化した前提条件と変更前後の値、START/STOP の phase、STOP 時の START 送信後の経過 ms、player y/vy を示す（診断用のみで比較条件には追加せず、文面は API 契約ではない。vy が得られない場合は n/a）。
単発診断用の `start_mining` と異なり、`dig` は元接続を永久に採掘専用へ隔離せず、
同じ接続で次の操作を呼べる。戻り値は server ACK や除去した actor の証明ではない。
START の後で呼び出しを止めた場合、接続が所有する task が元の対象へ ABORT を一度だけ送る。
採掘条件の変化で STOP を拒否した場合も同じ cleanup を使う。元の world/mode が変わった場合は
新しい文脈へ ABORT を送らず、cleanup を `ContextChanged` として記録する。
I/O が不確実な場合は自動再送・再接続せず、同じ world の次の採掘を拒否する。
`dig_cleanup_record()` は取消・拒否後の `Pending` / `Aborted` / `ContextChanged` / `Failed` を
切断後も読める診断で、server の ACK・採掘除去・再実行権限ではない。
ABORT の拒否・書込失敗は `ErrorKind::OperationUnconfirmed` を返し、元の STOP 拒否理由と
cleanup 失敗を診断チェーンへ保持する。呼出側は変更操作を停止して原記録を検査する。
ABORT が完全送信された場合は元の拒否エラーを返す。次の採掘を未解決 cleanup で拒否する場合も
`OperationUnconfirmed` であり、入力検査だけの `State` と区別できる。
同じ接続の運用採掘は一度に一つで、active な運用採掘の間は診断 mining の新規開始も拒否する。
STOP が完全送信済みか START だけで瞬時採掘が完了した後には、取消による ABORT を追加しない。
1.16.1 では送信済み STOP によって server が遅延破壊を予約すると、ABORT でもその予約は取り消せない。
1 秒の観測 timeout の後も含め、block が後から壊れることがある。

## 掘る時間の計算

公式の`BlockStateBase.getDestroyProgress`と`Player.getDestroySpeed`をそのまま計算する（f32）。

- 進み方 = 速さ ÷ 硬さ ÷ （正しい道具なら30、そうでなければ100）。硬さが負のblock（岩盤など）は拒否する。
- 硬さ・正しい道具が要るか・各itemの既定の速さと正しい道具かどうかは、両版の公式server jarで全block state
  （1.16.1は17,104、1.21.11は29,671）について公式のgetterを呼んで書き出した表による
  （`scripts/export_dig_profiles.py`、`data/client_api/dig_profiles-{版}.json.gz`）。
- 速さに掛かるもの:
  - 効率強化: 1.16.1は手に持ったitemのNBTの`Enchantments`（速さが1より大きいとき、レベル² + 1を足す）、
    1.21.11は受信した`minecraft:mining_efficiency`属性。
  - 採掘速度上昇・コンジットパワー（大きい方）: ×(1 + 0.2 × (レベル + 1))。採掘速度低下: ×0.3・0.09・0.0027・0.00081。
  - 1.21.11の`minecraft:block_break_speed`属性。
  - 目が水中: 1.16.1は兜に水中採掘がなければ÷5、1.21.11は`minecraft:submerged_mining_speed`属性（既定0.2）を掛ける。
    目が水中かどうかは、受信した目の高さのblockの水位で決める。
  - 地面にいない: ÷5。継続操作の実行中はそのframe、それ以外は各版の自分の状態の接地を使う。
- 属性と効果は受信した値（[自分の状態](common-player-facts.md)）。受信していない属性はclientの既定値を使う。
- 手に持ったitemの「速さ」は既定のitemの値で、itemごとに付け替えた`minecraft:tool`部品（1.21.11）などは反映しない。

## 実サーバーでの確認（2026-10-08）

公式`server.jar`（1.16.1・1.21.11）をoffline-mode・peacefulでlocalhostに起動し、`examples/dig_probe.rs`で
consoleから置いたblockを掘った。両版とも、すべてserverが壊した（`removed`）。

| block・道具 | tick | 1.16.1の所要 | 1.21.11の所要 |
| --- | --- | --- | --- |
| 土・素手 | 15 | 1.00 s | 1.03 s |
| 石・素手（正しい道具ではない） | 151 | 7.62 s | 7.65 s |
| 石・木のつるはし | 23 | 1.22 s | 1.24 s |
| 石・ダイヤのつるはし | 6 | 0.38 s | 0.40 s |
| 石・効率強化Vのダイヤのつるはし | 2 | 0.17 s | 0.19 s |
| 黒曜石・ダイヤのつるはし | 188 | 9.47 s | 9.50 s |
| 草（硬さ0） | 0（STARTで壊れる） | 0.02 s | 0.04 s |
| 石・木のつるはし・採掘速度上昇II | 17 | 0.93 s | 0.95 s |
| 鉄鉱石・木のつるはし（正しい道具ではない） | 151 | 7.63 s | 7.65 s |
