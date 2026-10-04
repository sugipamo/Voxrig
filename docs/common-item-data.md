# 共通item dataの受信

`Client::player_state()`、`Client::capture()`と`Client::screen_state()`のitemは、
setupで選んだ版のID・名前・count・実受信dataを保持する。同じ公開型を両版で使う。
未知・未対応dataをEmptyやdefault itemへ置き換えない。

`ItemData`の意味は次のとおり。

- `Default`: legacyのNBTがない、またはmodernの追加/削除patchが空。
  modern itemのprototypeが空という意味ではない。
- `LegacyNbt { bytes }`: root tag byteを含む元のlegacy NBT。constructorの`Damage=0`も保持する。
- `ModernComponents { patch }`: 元のnative component patch。`added`は型と値bytes、
  `removed`は明示的に削除された型。削除と、値が空bytesになるunit componentは異なる。

`ItemComponentPatch`は元のwire順序を保存する。等価比較は追加型/削除型の順序を無視するが、
各型の版・namespace・名前・値bytesは検査する。NBTのkey順序やtextの異なる表現まで
意味を正規化するものではない。prototypeとの統合、属性の解釈、stack容量の再計算、
native cursor hashの作成は別の版別規則を必要とする。

```rust,no_run
use voxrig::client::prelude::*;
use voxrig::client::SlotKnowledge;
# async fn inspect(client: &Client) -> Result<()> {
let player = client.player_state().await?;
if let Some(observed) = &player.inventory.slots[9] {
    if let SlotKnowledge::Item { item } = &observed.value {
        match &item.data {
            ItemData::ModernComponents { patch } => {
                for component in &patch.added {
                    println!("{}: {} native bytes", component.definition.name, component.bytes.len());
                }
                for removed in &patch.removed {
                    println!("removed: {}", removed.name);
                }
            }
            ItemData::LegacyNbt { bytes } => println!("{} legacy NBT bytes", bytes.len()),
            ItemData::Default => {},
            _ => {},
        }
    }
}
# Ok(())
# }
```

## Component registry

`Registry::item_component` / `item_component_by_native_id` /
`item_component_definition`は公式1.21.11の全104型を扱う。
IDは`RegistryKind::ItemComponent`に束縛され、item ID・block-state IDとは別。
未知ID/名前や別namespace/版のIDは拒否する。1.16.1にはこのregistryがなく、NBTを使う。
registryへの掲載と値codecの実装状況は別である。

## 現在の受信範囲

modernの追加値は、公式codecへ照合した52型に対応する。
scalar・unit・NBT形式の値、各native enum、lore、custom model data、tooltip display、
block-state property map、use effects、weapon、cooldownを元bytesのまま保持する。
custom data/NBTと名前を含むtext componentも全payloadを保存する。
削除patchは全104型に対応する。追加/削除の重複、未知ID、過大count、truncation、
不正なbooleanやNBT、1 MiBを超えるpatchは拒否する。

通常のclientbound item codecにはcomponentごとの長さがない。
slotの末尾やpacket残部を一つのopaque値として扱うと、後続slot/cursorまで取り込んでしまう。
対応値は型別に境界を読み取り、元bytesを保存してから後続fieldへ進む。
bundle/charged projectiles/containerなどの再帰的なitem、動的registry参照や
その他の構造化componentの一般codecは残作業。
既知だがまだ境界を読めない値では、従来どおり影響するbaselineを欠測にする。
通常full-content受信での部分更新は行わない。

この段階は受信・保持を追加する。既存のdefault-onlyクリック/転送/返却/設置等が、
component付きstackをdefaultと扱うことはない。
native-only default SWAPとdefault cursor hashも送信前に拒否する。
component付きitemの操作、任意legacy NBTの操作、crafting等は後続の統合作業で実装・検証する。

## 独立検証

`data/client_api/item_components-1.21.11.json`は未変更公式JARの実registry。
`item_component_cases-1.21.11.json`は全104 removal patch、95 component/item codec往復、
475 clientbound packet往復と複数型のmixed patchを保存する。
候補JSONは公式persistent codecでnative値に変換し、transient enumは元のby-ID関数で取得する。
codecは元のdecode/reencodeで全bytes一致と末尾消費を確認する。
game method、JAR、codecを置き換えない。実サーバーでの新component workflowの証拠とは区別する。

Rust側では52型の値と全removal/mixed patchを照合し、470対応packetについて
player/storage slot・後続item・cursor・元の受信ordinalと全prefix/trailing拒否を検査する。
別の共通Client consumerはlegacy NBTとmodern patchを同じ公開読み出しで検査する。

再生成は一つのJVMずつ、heap 512 MiB・CPU 1で実行する。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/export_item_components.py \
  --downloads .local/native-client-unification/downloads \
  --modern-classpath-file .local/integration-validation/client-api-unification/storage-outline-modern-classpath.txt \
  --runtime-output .local/integration-validation/client-api-unification/item-data-oracle/final
```

`--normalize-only --check`は保存済みraw出力とpackage data/source digestを照合する。
公式JAR・mapping・classpath、元requestと自作generatorのhashは
`data/client_api/item_component_source.json`へ束縛する。
