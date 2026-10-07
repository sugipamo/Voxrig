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
| `data/client_api/movement_oracle.json` | 場面と、両版のtickごとの位置・速度・接地・衝突・ダッシュ・姿勢 |
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

## 確認済みの範囲（2026-10-07）

既存の共有モデルが対応している乾いた地形の8場面（歩行・斜め・後退・その場ジャンプ・
ジャンプを押し続けて歩く・壁・ハーフブロックへの段差・縁から落ちる）で、両版とも全tickの位置と速度が
**bit単位で一致**した。

この比較で、1.16.1のモデルに2つのずれ（最後の1bit）が見つかり、直した。
- 入力ベクトルの正規化: 公式は長さで**割る**。逆数を掛けていた。
- 位置の更新: 1.16.1は移動のたびに衝突箱を動かし、位置を箱の中心から読み直す。
  位置に移動量を足していた（版の規則`position_from_bounds`）。

以前の部品ごとの比較（`VerifyLegacyDryMovement.java`）は1e-10の誤差を許していたため、この2つを見逃していた。
