# 移動の比較基準（movement oracle）

2026-10-07。共有物理エンジン（[設計メモ](physics-design.md)）の各段階を、公式実装の移動と
tickごとに比べるための基準。

## 仕組み

- 公式サーバーJARを**変更せずに**起動し、そのworldの中で「clientと同じように動くplayer」を毎tick動かす。
  移動の本体（`LivingEntity.aiStep`／`travel`、`Entity.move`、衝突・段差・摩擦・block効果）は公式コードがそのまま走る。
- playerは公式`Player`の小さな派生クラスで、clientの`LocalPlayer`が行う入力処理だけを移植している
  （キー→入力ベクトル、しゃがみ係数、ダッシュ開始・停止の条件、壁からの押し出し、水中でしゃがむと沈む）。
  移植した箇所はソースに注記した。ダブルタップによるダッシュと飛行は扱わない（ダッシュはキーで要求する）。
- サーバー側で動かしても結果は変わらない。移動に関わる処理で版の側（client/server）を見る分岐は、
  chunkが読み込まれていれば同じ結果になる（音・particle・燃焼など移動に関係しないものだけが違う）。
- 公開されている名前（Mojangのmappings）で書いたharnessを、手元で名前を付け直したJARに対してcompileし、
  compile結果だけを公式の難読化名へ戻してから、公式JARと一緒に実行する。ゲームのコードはリポジトリに含めない。

| ファイル | 内容 |
| --- | --- |
| `scripts/movement_oracle/java_1_16_1/MovementOracle.java` | 1.16.1のharness |
| `scripts/movement_oracle/java_1_21_11/MovementOracle.java` | 1.21.11のharness |
| `scripts/movement_oracle/scenarios.py` → `scenarios.json` | 場面（地形・開始位置・tickごとの入力・属性・effect） |
| `scripts/movement_oracle/run.py` | 入力の検証・名前の付け直し・compile・実行 |
| `data/client_api/movement_oracle.json.gz` | 場面と、両版のtickごとの位置・速度・接地・衝突・ダッシュ・姿勢 |
| `src/client/survival/oracle_tests.rs` | Rust側のエンジンとの比較 |

数値は`Double.toString`の10進文字列（往復で同じ値になる最短表記）で保存し、Rust側は`str::parse`
（正しく丸める）で読む。`serde_json`の数値の読み取りは最後の1bitがずれることがあるため使わない。

## 再生成

```bash
python3 scripts/movement_oracle/scenarios.py
python3 scripts/movement_oracle/run.py --downloads DIR --work WORKDIR
```

`DIR`には次を置く（`run.py`がhashを照合する）。

| ファイル | 出所 | hash |
| --- | --- | --- |
| `1.16.1-server.jar` | Mojang | sha1 `a412fd69db1f81db3f511c1463fd304675244077` |
| `1.16.1-server-mappings.txt` | Mojang | sha1 `11120c39da4df293c4bd020896391fb9ddd6c2ba` |
| `1.21.11-server.jar` | Mojang | sha1 `64bb6d763bed0a9f1d632ec347938594144943ed` |
| `1.21.11-server-mappings.txt` | Mojang | sha1 `5621e9253f05fd57872bbe7f8ddf5f9a7d525955` |
| `SpecialSource-1.11.4-shaded.jar` | Maven Central（`net.md-5:SpecialSource`） | sha256 `e2cab24b…c78518` |

1.21.11のJARはbundle形式で、中のゲーム本体とlibraryは`META-INF/*.list`のsha256で照合して取り出す。
JDK 21で確認した。両版で約1分。サーバーは`localhost`だけで、外部へは接続しない。

## blockの監査

同じ`run.py`は、各版の全block stateについて、衝突形状（entityの衝突が使う形と同じ呼び出し）・液体の有無・
窒息判定・位置による形の違いと、blockごとの移動に関わる処理を実装しているクラス・摩擦などの係数・tagを
`data/client_api/movement_blocks-{版}.json`へ書き出す。処理の名前は公式のmappingsから引く。

## 確認済みの範囲（2026-10-07）

既存の共有モデルが対応している乾いた地形の8場面（歩行・斜め・後退・その場ジャンプ・
ジャンプを押し続けて歩く・壁・ハーフブロックへの段差・縁から落ちる）で、両版とも全tickの位置と速度が
**bit単位で一致**した。

この比較で、1.16.1のモデルに2つのずれ（最後の1bit）が見つかり、直した。
- 入力ベクトルの正規化: 公式は長さで**割る**。逆数を掛けていた。
- 位置の更新: 1.16.1は移動のたびに衝突箱を動かし、位置を箱の中心から読み直す。
  位置に移動量を足していた（版の規則`position_from_bounds`）。

以前の部品ごとの比較（`VerifyLegacyDryMovement.java`）は1e-10の誤差を許していたため、この2つを見逃していた。

P3・P5の共有エンジン（[説明](physics-engine.md)）は、アイテムの使用中の移動を加えた162場面（両版で310回）のすべてでbit単位で一致した。
液体の場面では、サーバーのtickを進めないので、置いた水や溶岩は流れ出さない（指定した状態のまま比べる）。

## 登りの比較（2026-10-08）

`climbing_scenarios.py`で46場面を生成し、同じharnessで1.16.1の42場面と1.21.11の46場面を実行した。
結果は`data/client_api/climbing_oracle.json.gz`へ別に保存している。位置・速度・落下距離・接地・横の衝突・
ダッシュ・しゃがみ・水中・泳ぎをbit単位で比較し、すべて一致した。登りのテストは拒否も失敗として扱う。

```bash
mkdir -p .local/climbing/blocks
python3 -B scripts/movement_oracle/climbing_scenarios.py .local/climbing/scenarios.json
python3 scripts/movement_oracle/run.py \
  --downloads /absolute/path/to/downloads --work "$PWD/.local/climbing/oracle" \
  --scenarios "$PWD/.local/climbing/scenarios.json" \
  --output "$PWD/data/client_api/climbing_oracle.json.gz" \
  --blocks-output "$PWD/.local/climbing/blocks"
cargo test --locked --lib client::physics::oracle_tests::climbing_reproduces_every_official_tick_without_refusals
```

通常のJDK 21（`java`・`javac`・`jar`）が必要。サーバーは127.0.0.1へbindする。
場面には意図的に近隣の支えがない梯子や、実際のblock更新なら変化する足場の状態も含む。
block更新を進めず、受信済みの指定状態に対するclientの移動だけを比較している。


## ボートの比較（2026-10-08）

`boat_scenarios.py`が17場面を生成する。両版の公式`Boat`／`AbstractBoat`の
状態判定・浮力・操縦・`Entity.move`を、そのまま呼び出して比較する。
9通りの前後左右、初期yaw、neutral後の惰性、水中への落下、壁、石と氷の摩擦を含む。
人工の操縦者だけを乗せ、前の場面のentityが衝突に混ざらないように除去する。
全34実行の位置・速度・回転・角速度・接地・水面接触・パドルが全tickで一致した。

```bash
python3 scripts/movement_oracle/boat_scenarios.py .local/climbing/boat-scenarios.json
python3 scripts/movement_oracle/run.py \
  --downloads /absolute/path/to/downloads --work "$PWD/.local/climbing/boat-oracle" \
  --scenarios "$PWD/.local/climbing/boat-scenarios.json" \
  --output "$PWD/data/client_api/boat_oracle.json.gz" \
  --blocks-output "$PWD/.local/climbing/boat-blocks"
cargo test --locked --lib every_supported_boat_tick_matches_unchanged_official_methods
```

これはボート自身のclient計算の比較で、サーバーとの同期や一般entity衝突の再現を意味しない。
サーバー補正を受けた有限操作は中断する。実接続は[共通乗り物](common-vehicles.md)の手順を使う。

## 泡の柱とボートの液体操作（2026-10-09）

`fluid_control_scenarios.py`が泡の柱21場面、ボート33場面（従来の17場面を含む）を生成する。
泡の柱は上下、内部と水面、上の空気・水・トーチ・屋根、落下進入、横からの出入り、
ジャンプ・しゃがみ・泳ぎ、複数の柱と上下の柱が混在する接触を比較する。
ボートは水源・流水・落下中の水への水没、操縦とneutral、横方向と下方向の水流、
流水への落下、屋根の下での水面への移行を含む。
両版の公式処理を計108回実行し、拒否を許さず、全tickの位置と速度をbit単位で比較する。
playerは落下距離と移動状態、ボートは回転・角速度・接地・水の状態・パドルも比較する。

ボートのharnessは状態判定の後、公式`Entity.baseTick`を呼んで水流の押しを適用し、
浮力・操縦・移動を呼ぶ。非player entityでは平均した水流を正規化してから押す。
サーバー側だけの強制下車はこのclient計算へ混ぜず、別の実接続で確認する。
指定したblock stateを保つoracleと、実際に液体の更新が進む実接続は別の検証である。

```bash
python3 -B scripts/movement_oracle/fluid_control_scenarios.py .local/climbing/fluid-scenarios.json
python3 scripts/movement_oracle/run.py \
  --downloads /absolute/path/to/downloads --work "$PWD/.local/climbing/fluid-control-oracle" \
  --scenarios "$PWD/.local/climbing/fluid-scenarios.json" \
  --output "$PWD/data/client_api/fluid_control_oracle.json.gz" \
  --blocks-output "$PWD/.local/climbing/fluid-control-facts"
cargo test --locked --lib bubble_columns_reproduce_every_official_tick_without_refusals
cargo test --locked --lib submerged_and_flowing_boats_match_unchanged_official_methods
```


## ボートの泡の比較（2026-10-09）

`boat_bubble_scenarios.py`の16場面を両版の元のボート移動で実行する。
1.21.11では`AbstractBoat.tick`の順序に従い、`Entity.move`の後に元の
`Entity.applyEffectsFromBlocks`を2回呼ぶ。タイマーやサーバー側のlaunchを移植しない。
`received_boat_velocity`は元の移動への計測用入力であり、packet受信の証拠ではない。
実際の速度通知・強制下車は別の公式サーバー接続で検証する。

```bash
python3 -B scripts/movement_oracle/boat_bubble_scenarios.py .local/boat-bubbles.json
python3 -B scripts/movement_oracle/run.py \
  --downloads /absolute/path/to/downloads --work "$PWD/.local/boat-bubble-oracle" \
  --scenarios "$PWD/.local/boat-bubbles.json" \
  --output "$PWD/data/client_api/boat_bubble_oracle.json.gz" \
  --blocks-output "$PWD/.local/boat-bubble-blocks"
cargo test --locked --lib bubbles_and_velocity_changes_match_original_boat_movement
python3 -B scripts/run_boat_bubbles.py --accept-eula \
  --jars /absolute/path/to/downloads --binary target/debug/examples/climbing_control_probe
```

混在する泡、移動途中の接触、水浸しの上面を含む32実行の全tickについて、位置・速度・
回転・角速度・接地・水接触・パドルを比較する。モデルをpacketの現在位置やACKとは扱わない。


## ボートの特殊地形の比較（2026-10-09）

`boat_hook_scenarios.json`はスライムとベッドへの落下・前進・斜め移動、蜂蜜の床と側面、
クモの巣への接地・落下を含む14場面である。公式の非LivingEntityのcallbackを呼び、
28実行・1,470tickの全frameを比較する。計測入力の速度は受信証拠ではない。

```bash
python3 -B scripts/movement_oracle/run.py \
  --downloads /absolute/path/to/downloads --work "$PWD/.local/boat-hooks-oracle" \
  --scenarios scripts/movement_oracle/boat_hook_scenarios.json \
  --output "$PWD/data/client_api/boat_hooks_oracle.json.gz" \
  --blocks-output "$PWD/.local/boat-hooks-blocks"
cargo test --locked --lib special_block_hooks_match_original_nonliving_boat_callbacks
```

`--work`の版別ディレクトリは再生成されるため、他の検証出力とは別のパスを使う。
