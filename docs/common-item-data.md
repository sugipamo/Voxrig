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

## 共通custom metadata

`item.custom_data()?`は両版で`Option<NbtData>`を返す。legacyはconventionalな
`Damage`や`display`を含む完全なtag、modernは`minecraft:custom_data`を読む。
固定したmodern公式1,505 itemのdefault prototypeにcustom dataがないことを確認した。
未設定・明示的な削除・defaultは`None`、空compoundは`Some`で区別する。
ID・名前・版を検査し、元の`ItemData` bytesと外側の`ObservedValue.source`は保持する。

```rust,no_run
use voxrig::client::prelude::*;
# fn inspect(item: &ItemStack) -> Result<()> {
if let Some(data) = item.custom_data()? {
    if let Some(marker) = data.root().get("VoxrigProbe").and_then(NbtValue::as_int) {
        println!("marker = {marker}");
    }
}
# Ok(())
# }
```

`NbtValue`は12種の型、数値の幅、arrayとlistの違いを保持する。compoundのkeyは
UTF-16順で、重複はnativeと同じ最後の値を採用する。modern list内の単一empty-key
compound wrapperはnativeと同じ一段の展開を行い、legacyはcompoundを保持する。
空listのsubtypeとroot名は論理値に含めないが、元bytesは失わない。
modified UTF-8のNUL・補助文字・単独surrogateをUTF-16として保持する。
`NbtString::text()`は単独surrogateでエラーになり、JSONは通常stringまたはUTF-16配列を使う。

nativeのzero factoryによる正負zeroの統一とNaNのbitsを扱う。
`native_equivalent()`は同じ版に限定する。legacyで独立にdecodeしたNaNは元bytesが
同じでも比較不一致になり、同一の共有値はidentityで一致する。modernはrecordの比較規則を使う。
`persistent_crc32c()`はmodernの元`CompoundTag.CODEC`と`HashOps.CRC32C`による
**NBT値だけのhash**で、legacyは`None`。型marker・数値幅・UTF-16長と内容・list順序・
mapの子hash順序を照合する。modernでnative比較が一致するNaNでもpayloadが異なると
この純粋hashは異なる。serverのcache、item prototype・component patch全体のhash、
inventory操作の許可としては使えない。

decodeは1 MiB・深さ64・65,536 nodeに制限し、不正入力・truncation・末尾bytesを拒否する。
未変更公式JARの各版93値・4,371比較pair・10不正入力と、modernの93 hashを
`nbt_semantics-*.json`に保存して照合した。入力・tool・JARのdigestは
`nbt_semantics_source.json`に記録し、native method bodyやJARは配布しない。
再生成は`scripts/export_nbt_semantics.py`、保存済み出力との照合は`--normalize-only --check`で行う。
oracleは各JVMを512 MiB・CPU 1で順番に実行する。
実vanilla 1.16.1/1.21.11のsurvival/creativeでも同じgetterを使い、typed markerと
独立RCONの値を照合した。元item bytes・実受信sourceとreadonly frameを
`data/client_api/nbt_semantics_native_evidence.json`に保存した。実入力はcf1ae42に
staged変更を加えたもので、実行後のevidenceを実行時入力へ付け足していない。
一般itemの意味・text・prototype統合・slot規則とdata付き操作は引き続き追加対応が必要。

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

## 共通item properties

`item.properties()?`は両版で`ItemProperties`を返す。`client.registry().item(...)`の
default定義と異なり、現在のitemのdefault値・追加patch・明示的削除・受信時補正を使う。
`max_stack_size`・`max_damage`・`damage`・`damageable`・`damaged`・`stackable`を
同じ名前で読める。元bytesと外側の受信sourceは保持し、新たな受信を主張しない。
propertyはitem dataと固定native prototypeから導く値であり、外側のsourceは元itemの
根拠である。default容量等の独立したpacket受信やserverの操作受理へ読み替えない。

```rust,no_run
use voxrig::client::prelude::*;
# fn inspect(item: &ItemStack) -> Result<()> {
let properties = item.properties()?;
println!("capacity = {}, damage = {}", properties.max_stack_size, properties.damage);
# Ok(())
# }
```

legacyではitemによるNBT設定時のDamage補正、NBT numeric getterの幅変換・浮動小数の
floor、Unbreakableのbyte判定を使う。非damageableなstoneのDamage metadataも消さない。
modernではmax stackの削除はfallback 1、max damage/damageの削除はfallback 0。
max damage・damageの存在とunbreakableの非存在でdamageableを判定するため、
値0とcomponent削除は区別する。damage getterのclampは負の最大値でも元の分岐順を使う。
stackableは容量に加えてnativeのdamageable/damaged条件を使う。

値はnativeのsigned i32で保持する。元stream codecは負の容量や範囲外の値もdecodeできるため、
受信値をdefaultや正数へ置き換えない。このgetterはslot受入れ・有効な操作容量・
itemの意味の一致・inventory/cache hash・操作許可を証明しない。
公的なpatchを直接作った場合もID/名前/版/重複と全値のfield境界を検査する。
AIR/empty sentinelを通常のitem capacityとして扱わず、未知ID/dataも拒否する。

`item_properties-*.json`は未変更公式JARの975 legacy・1,505 modern default item
（各版のAIR sentinelを含む）と、実item stream decoderで読んだ1,428 legacy・1,288 modern
dataケースのnative getter結果。modernの3,490種類のprototype component値も元codecで保存する。
prototype内のraw registry参照は固定vanilla oracleのcontextに属し、実接続へIDを注入しない。
今回のproperty getterが使うprototype値は参照のないscalar/presenceだけ。
全componentの意味・item比較/hash・slot規則とdata付き操作は残作業である。
tool/input/JAR digestは`item_properties_source.json`に保存し、再生成は
`scripts/export_item_properties.py`、保存出力照合は`--normalize-only --check`で行う。
JVMは512 MiB・CPU 1で順番に実行する。
両版・両modeの実vanillaでも同じgetterを使い、legacy stoneのDamage=7と
modern stoneのmax stack size=16を独立RCONへ照合した。受信bytes・元itemのsource・
readonly frame・実行時入力は`data/client_api/item_properties_native_evidence.json`に保存する。
実入力は4105a49にstaged変更を加えたもので、実行後のevidence/docsを入力へ付け足さない。

## Component registry

`Registry::item_component` / `item_component_by_native_id` /
`item_component_definition`は公式1.21.11の全104型を扱う。
IDは`RegistryKind::ItemComponent`に束縛され、item ID・block-state IDとは別。
未知ID/名前や別namespace/版のIDは拒否する。1.16.1にはこのregistryがなく、NBTを使う。
registryへの掲載と値codecの実装状況は別である。

## 現在の受信範囲

modernの追加値・削除patchは、公式1.21.11の全104型の値境界に対応する。
scalar・unit・NBT、native enum、struct・optional・list/map、registry参照、
名前付き入れ子item、effectの再帰、predicate/表示形式のdispatcherを元bytesで保持する。
公式codecの組み合わせを651 nodeの固定schemaへ記録し、各型のnative classと照合する。
追加/削除の重複、未知ID/dispatcher、過大count、truncation、不正なboolean/NBTは拒否する。
値のbyte上限1 MiB、list/map上限65,536、共有work budget65,536、codec深さ256を設ける。
この制限を超える受信を部分的な既知itemとして公開しない。

通常のclientbound item codecにはcomponentごとの長さがない。
slotの末尾やpacket残部を一つのopaque値として扱うと、後続slot/cursorまで取り込んでしまう。
対応値は型別に境界を読み取り、元bytesを保存してから後続fieldへ進む。
bundle/charged projectiles/containerは後続slotまで取り込まず、各入れ子itemのID・patchを
同じbudget内で読み取る。原形を保持するため、入れ子値も外側componentの元bytesへ含める。

registry参照は元の数値/inline/tag表現を保持する。
現在の接続のdatapackによる名前・値への解決や、predicate/属性の意味の検証とは別である。
検証fixtureのvanilla registry IDsを実接続へ注入したり、名前を推測したりしない。
実受信のentry lookupは`Client::server_registry_state()`に追加した。
`ServerRegistryId`をconnection/configurationへ束縛し、再設定後の旧IDを拒否する。
一般componentのinline/tag表現の解釈やlegacy codecの個別entry resolverは引き続き別作業。
詳細は[接続先から受信したregistry](common-server-registries.md)を参照。
prototypeとの統合、一般NBT/text等価性、hash・容量・slot規則と共通property getterは残作業。

この段階は受信・保持を追加する。既存のdefault-onlyクリック/転送/返却/設置等が、
component付きstackをdefaultと扱うことはない。
native-only default SWAPとdefault cursor hashも送信前に拒否する。
component付きitemの操作、任意legacy NBTの操作、crafting等は後続の統合作業で実装・検証する。

## 独立検証

`data/client_api/item_components-1.21.11.json`は未変更公式JARの実registry。
`item_component_cases-1.21.11.json`は全104 removal patch、518 component/item codec往復、
2,590 clientbound packet往復と複数型のmixed patchを保存する。
候補JSONは公式vanilla resource・registry・tag loaderの元contextでnative値に変換する。
実itemのprototypeも元native値として取得し、transient enumは元のby-ID関数で取得する。
保存した133 registryのID/名前はこのdefault vanilla fixtureの事実で、任意実接続のbindingではない。
codecは元のdecode/reencodeで全bytes一致と末尾消費を確認する。
game method、JAR、codecを置き換えない。codec往復と、下記の実サーバー上の受信workflowは別の証拠。

Rust側では全104型の値と全removal/mixed patchを照合し、2,590 packetについて
player/storage slot・後続item・cursor・元の受信ordinalと全prefix/trailing拒否を検査する。
別の共通Client consumerはlegacy NBTとmodern patchを同じ公開読み出しで検査する。

未変更vanilla 1.16.1・1.21.11を順に起動し、両版のsurvival/creativeで名前付きstoneを
同じ公開Clientから観測した。外部fixtureの後にbaselineを超える実受信slot ordinal、
同じsession/world、名前・個数・`VoxrigProbe` markerを確認する。
modernは元の`custom_data`/`custom_name` patch、legacyは元NBTを保持する。
RCONが個数・名前・markerを独立に照合し、位置/回転は不変、readonly traceには
外向きclick/CLOSE/creative-slot変更がない。
実受信packetをRustのpatch decoderでも読み直し、共通観測との全bytes一致を検査する。
元report/trace/logと実行前source/binary hashは
[`item_data_native_evidence.json`](../data/client_api/item_data_native_evidence.json)に保存した。
実行後に追加したevidence/tests/docsを実行入力へ後付けしない。

初回のmodern実行は切断時にheaderのみの不完全な送信frameを検出し、失敗として保存した。
短いprotocol応答のheaderと本文を一つのbufferで送るよう修正し、最終の両版は
trace errorなし・JVM exit 0で完了した。これはTCP送信をatomicとする保証ではなく、
途中送信や取消の不確実性は従来どおり保持する。
明示した切断後の完全なclientbound frameを転送できない場合は「元frameは完全・未配達」と
記録する。途中frame・送信側の失敗・要求外の切断は引き続き検証エラーになる。
この境界は自作socket試験で検査し、最終実ゲーム試験には未配達frameもなかった。

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


追加の実ゲーム試験では、同じClientと両modeでmodernの名前付き入れ子item、
実エンチャント、本の本文も同時に受信し、個数・marker・level・本文をRCONへ照合した。
元SET_SLOTを再decodeして公開patchの全bytesと照合する。
実入力hashと先行fixture照合の失敗は
[`item_data_complex_native_evidence.json`](../data/client_api/item_data_complex_native_evidence.json)へ保存した。

固定schemaの組み合わせ・再帰先・元dispatcher全branchは
`item_component_schema-1.21.11.json`と`item_component_schema_source.json`に束縛する。
元のmethod bodyは配布しない。再生成は下記を使い、保存rawからの照合は`--normalize-only --check`。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/export_item_component_schema.py \
  --downloads .local/native-client-unification/downloads \
  --modern-classpath-file .local/integration-validation/client-api-unification/storage-outline-modern-classpath.txt \
  --runtime-output .local/integration-validation/client-api-unification/item-data-schema-oracle/current
```
