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
2. 1 tickごとの進み方を`ticks`回足して1に届くまで待つ（clientの時計。1 tickの余裕を足す）。
3. STOP_DESTROY_BLOCKを送る。serverは自分が数えたtickで70%以上進んでいれば壊す。
4. 受信したblockの状態が変わるのを最大1秒待ち、`removed`と`final_state`を返す。
   壊れなかったことはerrorではない（serverが決める）。1.21.11は`sequences`にSTARTとSTOPの操作番号が入る。

送信前に、受信したgame modeがsurvivalであること、対象がloadedで目から4.5以内であること、
未解決の操作がないことを検査する。STARTの後に呼び出しを止めた（futureを落とした）場合、ABORTは送らない。

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
