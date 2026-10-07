# 版ごとの定数表

版の違いは2つに分けて管理する。

- **値の違い** → `src/versions/table.rs`の定数表（`VersionTable`）。
- **手順の違い**（パケットの構造、接続の流れ、item dataの形式、物理の計算手順など）→ 各版のadapterと
  `client::adapter`のtrait（[共通trait設計](version-adapter-trait.md)）。

## 定数表に入っているもの

`MinecraftVersion::table()`が`&'static VersionTable`を返す。表は`const`で、項目を足すと
全版の値を書くまでコンパイルが通らない。

| 区分 | 項目 |
| --- | --- |
| 識別 | `name`（`MinecraftVersion::name()`）、`protocol`（`MinecraftVersion::protocol()`） |
| registry関数 | `native_state`、`state_id`、`collision_boxes`、`item_id`、`item`（各版の静的な関数ポインタ） |
| entity | `health_metadata_index`（8 / 9）、`equipment_slots`（native番号→共通の装備欄）、`dimensions`（生成表） |
| 値 | `generic_slot_class`（container `Slot`の難読化クラス名） |
| 物理 | `physics`（`PhysicsConstants`: 重力・抵抗・加速・ジャンプ・段差など）、`physics_rules`（`PhysicsRules`: 三角関数表、入力の正規化、微小速度の切り捨て、段差の探索手順など） |
| 同梱データ | 地形・rail・menus・かまど・cursor返却・クリック・転送・registry一覧・crafting menu・crafting外形・item属性・採掘道具・収納外形 |

同梱データを解析した結果は`PerVersion<T>`で版ごとに1回だけ作って保持する。

```rust
static SHAPES: PerVersion<Shapes> = PerVersion::new();
SHAPES.get(version, |table| parse(table.data.dry_terrain))
```

## 生成した表

大きな表はJSONからRustの`static`配列を生成し、生成ファイルをリポジトリに含める。

```bash
python3 scripts/generate_version_tables.py
```

| 生成ファイル | 元データ |
| --- | --- |
| `src/versions/java_1_16_1/generated.rs` | `data/entities.json` |
| `src/versions/java_1_21_11/generated.rs` | `data/java_1_21_11/entity_dimensions.json`（公式サーバーから書き出し） |
| 両版の`generated.rs`の`BLOCK_PHYSICS` | `data/client_api/block_physics-*.json`（公式サーバーから`ExportBlockPhysics.java`で書き出し） |

生成ファイルの先頭には元データのsha256を書く。`cargo test`の
`generated_tables_match_their_sources`が現在の元データと照合し、ずれていれば失敗する
（スクリプトを再実行して更新する）。実行時のJSON解析は不要で、表の名前順を二分探索する。

## まだ表になっていない値の違い

次は処理の違いと分けにくいため、現時点ではadapterまたは共通層の`match`に残している。

- item data: 1.16.1のNBTと1.21.11のcomponentの扱い（`nbt.rs`、`item_semantics.rs`、`received_items.rs`等）。
- パケットの組み立て: 乗り物の入力・降車（`vehicle/`）、entity移動差分の丸め（`entity/motion.rs`）。
- 接続と記録の再生（`connection.rs`、`recording.rs`）。

## 版を追加するとき

1. `MinecraftVersion`に版を足し、`table.rs`に`VersionTable`を1つ書く（不足はコンパイルエラーで分かる）。
2. 同梱データを`data/`に置き、必要なら`generate_version_tables.py`に生成元を足す。
3. adapterを実装し、`client::adapter`の各traitを実装する（不足はコンパイルエラーで分かる）。
