# 共通item dataの受信

`Client::player_state()`、`Client::capture()`と`Client::screen_state()`のitemは、
setupで選んだ版のID・名前・count・実受信dataを保持する。同じ公開型を両版で使う。
未知・未対応dataをEmptyやdefault itemへ置き換えない。

registryを参照するitemの検査には`Client::received_inventory()`を使う。
両adapterは実受信slot・cursorとregistryを同じロック境界で取得する。
`ReceivedInventory` / `ReceivedSlot` / `ReceivedItem`はClientだけが生成でき、
stack bytes、受信packet ordinal、接続・world・configurationの所有情報を読み取り専用で保持する。
予測やcompatibility cacheは含めない。legacyではcacheの変換やphysicsの取得にも依存しない。
slotの`None`は未受信、`SlotKnowledge::Empty`は明示的な空を意味する。
itemの有無は`ReceivedSlot::item()`で検査する。

```rust,no_run
use voxrig::client::prelude::*;
# async fn inspect(client: &Client) -> Result<()> {
let inventory = client.received_inventory().await?;
if let Some(item) = inventory.slot(9)?.and_then(ReceivedSlot::item) {
    println!("{} x{} received at {}", item.stack().name, item.stack().count,
        item.receive_sequence());
    let entry = item.registry_state().find_entry("minecraft:item", &item.stack().name)?;
    println!("{}", item.registry_state().entry_name(&entry)?);
}
# Ok(())
# }
```

取得後にClientが再接続・再設定しても古い値とregistryは変わらない。
再設定後のslotを古いconfigurationへ束縛したり、別途取得した最新registryを過去のitemへ
付け替えたりしない。capture ordinalは全slotがそのpacketで更新されたことを意味しない。
JSONは診断用であり、receiptへ戻すdeserialize/公開constructorは提供しない。
大きいregistry payloadはinventory全体に一度だけ出力し、各slotには所有stampを出力する。
receiptの取得だけではdata付き操作の許可やnative意味比較を証明しない。

## 受信itemの比較

`ReceivedItem::native_equivalent(&other)`は両版でcount・item種別・native dataを比較する。
同じconnection/configurationのreceiptに限定し、別の所有者を持つ値はエラーにする。
公開の`ItemStack`から比較の所有情報を作り直すことはできない。readonlyの比較であり、
slot受入規則、操作の成功、server synchronizer/cacheのhashを表さない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn compare(client: &Client) -> Result<()> {
let inventory = client.received_inventory().await?;
if let (Some(a), Some(b)) = (
    inventory.slot(9)?.and_then(ReceivedSlot::item),
    inventory.slot(10)?.and_then(ReceivedSlot::item),
) {
    println!("same native stack: {}", a.native_equivalent(&b)?);
}
# Ok(())
# }
```

legacyはitem constructorによるDamageのInt化・clamp・欠落時の挿入を行ってからNBTを比較する。
元bytesは変更しない。同じpacket・player slot/cursor・sessionのreceiptは同じdecoded値の
共有情報を保持し、別slot・cursor・別packetのNaNを同じbytesだからと同一にしない。
入力4,805件・3,595同値グループと独立decodeのnative比較へ一致した。

modernは1,505 itemのprototypeへ追加/削除patchを適用し、全fieldを型付き値へ変換する。
Mapの重複keyは最後を使い、map順序は比較しない。list順序・数値wrapper・codec/constructor・
NBT・text/profile/book・nested item/count・bundle Fractionを保持する。registry参照はそのreceiptの
builtin版または実受信ownerへ束縛し、tagには実際に宣言された名前付き集合を要求する。
JSON、元wire bytes、CRCを本番の比較値には使わない。

固定した元codec corpusの全104型・4,134 component入力・2,313,719比較、
13,853 item入力・12,723同値グループ・24,789 prototype操作、入れ子componentの66,430比較へ一致した。
original holder/holder-setの42値・903比較も追加し、stream/lookupの741比較を共通の参照・tag keyへ
照合した。独立したemptyNamed factoryは別objectであり、未宣言tagのstream decodeは元が拒否する。
これらは元JVM/保存済みcorpusと合成packet/TCPの試験であり、新たな実vanillaサーバーの試験ではない。

任意のtext内item/dialog constructor、tag再読込のlookup寿命、一般data付き在庫操作、
実server cache/slot規則は引き続き対応が必要。未解決constructorや不足したregistry/tagは
明示的なエラーにし、同じ・異なるitemと推測しない。corpus照合を未制限な全入力対応とは扱わない。

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

## 型を保持する内部field decoder

全104型の既存の651 node構造を、scalar・boolean・NBT・sequence・optional・list/map、
registry/holder/tag、profile、typed component、入れ子item/patch、dispatcherを区別する
内部value treeへ接続した。通常の受信はcaptureを無効にしてtreeを保持しない。
既存の共通property getterのsigned整数もこの同じgrammarから読む。
追加の公開low-level APIや、stack比較・操作許可はこの段階では作らない。

元factoryの19 enum nodeについて6,230 alias入力を検査し、zero fallback・clamp・wrapを区別する。
EquipmentSlotの宣言順とID順は異なり、tropical fish/rabbitは非連番IDを含む。
そのためenum ordinalや最大値からIDを推測しない。元ByIdMapとMath helperもlocalで確認した。
固定幅の5 nodeは元stream codecの実decoder/getterで型と34入力を確認し、
float/doubleのNaN・infinity・signed zeroを保持する。姿勢のfinite-only decoderへ流用しない。
NBTの20 unnamed rootは元NbtIo/NbtOpsへ照合し、全kind、EndTag、modified UTF-8、
重複compound key、modern list wrapperと純粋NBT hashを同じ共通decoderで読む。
数値streamのsigned zeroとNBT factoryのzero補正は区別する。

4,134 component入力を元canonical wireへ照合した。これはwire fieldの型・構造の検査で、
mapのnative等価性やforward codecによるconstructor/text正規化を証明しない。
registry/holder/tagは名前付きの未解決参照で、fixture IDやtag名から実server値を推測しない。
このtreeを使う完全なcomponent/prototype/text比較・persistent encoder・live registry/cache・
slot規則/data付き操作の実装は引き続き統合する。

runtimeは`component_value_rules-1.21.11.json`の小さな型定義だけを読み、
大量のprobeは`component_value_cases-1.21.11.json.gz`へ分離した。
`component_value_rules_source.json`にtool/JAR/classpath/mappingとraw/final digestを保存する。
`scripts/export_component_value_rules.py`で元JVM512 MiB・CPU1を使って生成し、
同じruntime directoryの`--normalize-only --check`で保存出力を照合できる。
このstandalone検査を新たなlive gameplay/ServerPlayer cache検証とは扱わない。

### Forward codecとIdentifierの正規化

内部treeは308のforward nodeのcodec identityも保持する。fieldを読み取っただけでは、
それをどのnative constructorが組み立てるか失われるためである。
通常受信ではこのwrapper/treeも保持しない。未実装constructorを正規化済みとは扱わない。

元Identifier streamのforward nodeは名前空間とpathへ分け、`stone`・`:stone`を
`minecraft:stone`として読む。不正な文字や複数colonは、treeを作らない受信経路でも拒否する。
元bytesは省略表記のまま保持する。空pathや`..`等は元factoryが受理するため、追加の変換をしない。
ASCII全128文字のnamespace/path境界、Unicode、UTF-16の不正surrogateを含む274入力について、
元1.16.1 ResourceLocation constructorと1.21.11 Identifier streamの受理・補正が一致した。
この共通内部grammarは名前の解釈だけを行い、registry値や実接続の参照は解決しない。

textは125候補の元NBT変換・stream decode・persistent encodeと、3,486のnative比較を保存した。
83候補は元codecが受理し、literal/empty、keybind、translate、score、selector、NBT、objectの
全8contents実装classを含む。失敗にはstream拒否と入力JSON→NBT変換/encodeの失敗があり、
`failure_stage`で区別する。候補をすべて有効なtextとして扱わない。
元component比較とItemStackのdata/matches比較では`red`と`#ff5555`が等しく、
`bold`未指定と`false`は異なる。元persistent encoderは前者の色表記を区別して保持する。
wire/NBT field一致やcanonical spellingを、そのままnative item identityやcache hashにしない。
これらは完全なtext/constructor比較の実装用oracleであり、runtimeのtext正規化・操作対応の完了ではない。

runtime定義は小さな`component_normalization_rules-1.21.11.json`だけを読み、
検査値・比較・forward catalogは`component_normalization_cases-1.21.11.json.gz`へ分離する。
`scripts/export_component_normalization.py`は公式JARとmappingを照合し、JVM512 MiB・CPU1で
modern/legacyを順に呼び出す。source recordは入力・tool・raw/final digestを束縛し、
同じruntime directoryの`--normalize-only --check`で保存出力を確認できる。
元method body・JAR・bytecode検査ログはlocalに留め、standalone oracleをlive検証へ読み替えない。

### 共通text fieldモデル

内部の共通textモデルへ8種類のcontents、11個のstyle field、順序付きsiblingsを読み取る。
modernのtext stream rootを明示的にcaptureした場合だけ生成し、通常受信の元bytes保持経路では生成しない。
translation引数は元JavaのByte/Short/Integer/Long/Float/Doubleを区別する。
styleのないliteral引数はstringへまとめるが、`bold: false`等の指定があればtextとして保持する。
float/double wrapper比較はNaNをまとめ、signed zeroを区別する。UTF-16は単独surrogateも保持する。

色はRGB比較値と元persistent表記を別々に保持する。16個の名前付き色と、実行したJDK
21.0.12.1の`Character.digit(char, 16)`が受け付ける394 code unitを元入力から固定した。
shadow RGBAは元Number.floatValue・floor・channel maskに合わせ、範囲外/NaN/infも検査する。
booleanは元Number.byteValueを使い、未指定とfalseを区別する。NBT contentsのinterpretとseparatorは
元lenient規則を使う一方、selectorのseparatorはstrictに読む。

公式codecへ182個のwire入力を与え、受理された149値の元contents/style getterと照合した。
元component.equalsの11,175 pair中、未解決の依存を含まない10,731 pairを内部field keyで照合し、
残る444 pairは比較キーを作らない。dialog、入れ子item、
追加のconstructor検査が必要なfieldを未完了として残す。
このキーは完全なconstructor validation、全component/item identity、persistent encoder、cache hash、
操作の許可を証明しない。未解決のdialog/item constructorを含む完全な構築規則は残作業である。

runtimeの小さな色grammarは`text_color_rules-1.21.11.json`、検査値と比較は
`text_core_cases-1.21.11.json.gz`、元入力/tool/getter/出力のdigestは`text_core_source.json`へ分離する。
`scripts/export_text_core.py`は未変更公式JARをJVM512 MiB・CPU1で呼び出す。
保存済みruntimeの`--normalize-only --check`でも生成物を検査できる。
この段階のstandalone検証は新しいlive Client検証やserver cache検証ではない。

### text constructorの候補選択

元FuzzyCodecは、native mapperの順にcandidateをdecodeし、最初の成功を採用する。
単に最初にあるkeyを選ぶ規則ではない。contentsはtext→translatable→keybind→score→selector→
nbt→object、NBT sourceはentity→block→storage、objectはatlas→playerの順を使う。
`type`・`source`・`object`がある場合はStrictEitherを使い、その明示候補に失敗してもfuzzyへ戻らない。
未指定の場合に失敗した候補の再帰処理も、同じwork budgetを消費する。
独自のdepth/work制限はnativeの意味エラーと区別し、fuzzyやlenient optionalでも無視しない。
制限超過を低優先候補への切替や有効separatorの省略へ変えないことを回帰検査する。

translationのfallbackは元lenient optional fieldで、不正な値を未指定として読む。
player objectのhatはstrict optional fieldで、不正な指定をdefault trueへ補正しない。
元text streamが許可しないOPEN_FILE clickは拒否する。
小さな`text_constructor_rules-1.21.11.json`は実native mapperから取得した順序だけを持つ。
元codecの成功値・拒否値・比較は`text_constructor_cases-1.21.11.json.gz`、入力/tool/元JAR/
classpath/raw/final digestは`text_constructor_source.json`へ分ける。

複数候補・逆のkey順・不正な先行候補・明示discriminator・strict/lenient optionalを含む419入力を
未変更公式codecへ与え、受理された354値の取得済みfieldと照合した。65拒否は内部projectionも拒否する。
以前保留したhover entityの不正UUIDも、下記の元codec検査に基づき拒否する。
元component.equalsの62,835組中、依存を含まない61,776組は一致し、1,059組は未解決である。
これらはcoreの182入力/10,731比較を含む拡張検査で、別々の独立した合計値へ加算しない。

dialog、入れ子itemの完全な構築が未実装のため、
それらの候補の妥当性と候補間のfallbackも全条件で一致するとは主張しない。
内部field projectionを完全なnative constructor検査やitem操作の許可に使わない。
通常受信の元bytes保持経路、新しいlive gameplay/cache検証の不在、残る全体統合の範囲は同じである。

### profile constructorの共通化

modernのprofile stream rootとtext内のplayer objectを共通の内部profileへ読み取る。
Dynamicの名前/UUID、Staticのfull/partialを区別し、元constructorが等価判定に使う種類を保持する。
名前・UUIDが両方あっても、NBTのfullと通信で明示されたpartialは同じ値にまとめない。
NBTの名前は16 UTF-16 unit以内の元ASCII grammarで検査する。通信の名前は同じ長さ制限だが、
Unicodeや空白も受け付ける。空文字は両方で許可される。

propertiesはcompact mapとrecord listを元codecの別々の制限で読む。キー間の順序は比較に使わず、
同じキー内の値の順番・重複・署名の未指定と空文字は保持する。NBTのcompact mapでは長い値を
受理する場合もあるため、constructorの成功と後続の通信encode失敗を分けて記録する。
元property keyのiteration順はoracle出力と元bytesに残し、比較用の並べ替えをpersistent encodingや
hashの代わりにしない。署名の通信読み取りは元codecの1,024 unit制限へ修正した。
skinはtexture/cape/elytraのIdentifierと元のcomputed texture path、wide/slimモデルを保持する。
componentのboolean streamは元ByteBufと同じく0以外をtrueとし、モデル値2/255も照合する。

未変更公式codecでNBT/通信の324入力を検査し、253受理値の全constructor fieldと
32,131組の元profile.equalsを照合した。正規化した通信で元codec自身が等価とした値も照合する。
単独surrogateを含む2値は元通信往復で等価にならないことを記録し、正常往復として扱わない。
現在のtextの比較範囲は上記の10,731組/61,776組へ広がった。以前の検査と重複するため加算しない。

`profile_rules-1.21.11.json`は元name/modelの小さなgrammar、`profile_cases-1.21.11.json.gz`は
入力・結果・比較、`profile_source.json`は元JAR/mapping/classpath/tool/raw/finalのdigestを保持する。
owned `scripts/export_profiles.py`はJVM512 MiB・CPU1で実行し、保存runtimeの
`--normalize-only --check`でも確認できる。通常受信でprofile modelを生成しない。
online profile/skin解決、persistent encoder、server cache、legacy profile等価判定、
data付きitemの操作許可や新たなlive gameplay検証はこの段階に含まれず、全体の統合は継続中である。

### selector constructorとscoreの共通化

modern textのselector patternを共通の内部constructorへ読み取る。元の21 options、適用順序と
重複条件、player name/UUID、範囲、scores/advancements、SNBT predicateのsyntaxを検査する。
scoreの名前は元constructorと同じく、selectorとして成功した値とliteralへ戻る値を区別する。
比較は元SelectorPattern.equalsが使うraw UTF-16 patternを保持し、解析後のpredicateを比較キーへ混ぜない。
元parserは入力末尾までの消費を要求しないため、消費cursorも元値へ照合する。
selector失敗時の下位fuzzy候補への切替を検査し、独自work/depth制限は切替で隠さない。

未変更公式codec/parserへ1,955入力を与え、998受理値と498,501組の元equalsを照合した。
selector、text、非canonical booleanを含むprofileの検査を含むため、上記のtext/profile比較へ加算しない。
optional/either/profile署名のpresence byteも元ByteBufと同じく0以外をtrueとして読む。
元getterから得た他のcompiled selector fieldはoracleへ残すが、entity queryの実行は未実装である。

SNBTは元のquoted escape、数値suffix/base/空白、typed array、bool/uuid operationを検査する。
名前付きescape用の288,767 character namesと10 Unicode uppercase foldは、実行したJDK
21.0.12.1から取得した事実として固定する。SNBTでは前後のcontrol空白をtrimした後に
ASCII文字・数字・hyphen・spaceだけを許可する。JDK自体のUnicode fold受理と混同しない。
名前catalogは該当escapeの内部検査時だけ読み取り、通常受信では構築しない。

`selector_rules-1.21.11.json`は小さなgrammarと元builtin entity type一覧、圧縮casesは入力・結果・
比較、source recordは元JAR/mapping/classpath/tool/request/raw/final digestを持つ。
`character_names-21.0.12.1.json.gz`とsource recordはJDK実行ファイル/modules/tool/raw/finalへ束縛する。
owned exporterはJVM512 MiB・CPU1で実行し、保存runtimeの`--normalize-only --check`でも確認できる。
ゲームのmethod body・JAR・JDK modules・inspection logは配布しない。
legacy selector、実world/entity解決、persistent/hash/cache、公開操作の許可や新しいlive gameplayの
証拠にはならず、残る全機能の統合は継続する。

### URL constructorの共通化

modern textのopen_url clickを共通の内部URI constructorへ読み取る。
未変更公式codecが使うJDK URIのraw UTF-16、scheme-specific part、opaque/hierarchical、
authority、user info、host、port、path、query、fragmentを保持する。
元codecの許可schemeはhttp/httpsで、host必須のHTTP接続用URLに置き換えない。
opaque URIやregistry authorityも元codecが受理する場合には保持する。

比較は元URI.equalsへ合わせ、scheme/hostのASCII case、percent escapeのhex case、
server portの数値を区別して扱う。registry authorityの文字case、pathのdot segment、
未指定と空文字、非escapeの文字とpercent表記、IPv6の表記差は同一へ潰さない。
比較キーはraw getterの表記を上書きせず、persistent encoderやhashへ使わない。

未変更公式URI/text codecへ1,810入力を与え、1,089受理値のgetter・入れ子click routeと
593,505組の元URI/component.equalsを照合した。ASCII・Unicode・単独surrogate、
IPv4/IPv6/scope、port、percent escapeと、hover/translation/selector separator/sibling/
lenient NBT separatorを含む。上記のtext検査と重複するため合計へ加算しない。
独自work制限はURLやlenient NBT separatorで隠さず、typed limitとして伝播する。

`uri_rules-1.21.11.json`は元ASCII mask・非ASCII exclusion・許可schemeの小さなgrammar、
圧縮casesは入力・getter・比較、source recordは元JAR/mapping/classpath/JDK executable/
modules/tool/request/raw/finalのdigestを持つ。owned exporterはJVM512 MiB・CPU1で実行し、
保存runtimeの`--normalize-only --check`でも確認する。module openingは観測用reflectionだけで、
元codec/parserのmethodを置き換えない。元のmethod body・ゲーム/JDK binaries・inspection logは配布しない。
通常受信ではURI modelを構築しない。URLを開く処理、DNS/HTTP接続、legacy URIの等価判定、
persistent/cache、公開item操作や新たなlive gameplay検証はこの段階の証拠に含めない。
残るconstructorと一般item/操作の全体統合は継続中である。

## Item比較・persistent hashの基礎

`item_semantics-*.json.gz`は未変更公式JARで独立にdecodeしたitemの比較結果。
legacy 4,805件、modern 13,853件についてcountを含むnative比較とcanonical wireを保存する。
modernでは全104型、4,134件のcomponent値をnative value比較・persistent codec・
original HashOpsへ通し、5,846 nodeの型付きhash入力を記録した。byte/short/int/long、
float/doubleの生bits、boolean、UTF-16 string、array、list、mapを区別する。
wire bytesをCRCへ通した結果ではない。原codecと観測wrapperの成否/hashが同じことも検査する。
Rustの共通内部hash計算は8,335件の直接・fresh・cache rootと照合し、既存のNBT getterもこの計算を使う。
fixtureによる値→hash lookupや、任意componentを読む公開hash APIにはしない。

modernの24,789件のprototype同値追加・不存在component削除は、元itemとnative比較が等しい。
したがって元patch bytesの差だけでitemの意味の不一致を断定できない。
legacyでは同じbytesを別々にdecodeしてもNaNを含むitemが不一致になり得る。
modernのcustom_dataではNaN payloadが異なってもnative値は等しく、直接hashは異なる一方、
native値をkeyにしたcacheは先に計算したhashを返す例を4件保持している。
CRC一致、native値の等価性、送信するcache hashは別の事実として扱う。

standalone検査は元のtyped encoder・RegistryOps・HashOps・Guava native-key cacheを
組み合わせた自作HashGeneratorを、元HashedStack creator/matcher/stream codecへ渡す。
cache容量256は元serverの値へ照合したが、実ServerPlayerのsynchronizer/cacheを実行した証拠ではない。
prototype内のregistry IDも固定oracle contextのもの。実接続のregistryへ流用しない。
13件は元persistent encoder/hashed-stack creator自身が拒否した結果を保存する。
非persistentなmap_post_processing/creative_slot_lockとscalarの範囲外値を含み、成功へ置き換えない。

元JAR/mapping/classpath、request、tool、raw/final出力digestは`item_semantics_source.json`に保存する。
`scripts/export_item_semantics.py`で両版をJVM512 MiB・CPU1で順に再生成する。
同じruntime directoryを指定した`--normalize-only --check`で保存出力を照合できる。
ゲームのJAR/method bodyとruntime worldは配布しない。この検査はgameplayの受理・slot規則を証明しない。
完全なcomponent意味比較、live registry解決、server cacheとの対応とdata付き操作の実装・検証は続く。

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
容量・耐久の共通property getterとtyped NBTは実装済み。
一般component/text等価性、完全なprototype統合、item/cache hashとslot規則は残作業。

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

### clickとentity tooltipの内部共通化

modernのrun_command/suggest_commandは元CHAT_STRINGのUTF-16規則を使い、
0〜31、127、167のcode unitを拒否する。clipboard/literalへこの制限を流用しない。
change_pageのNumber.intValueと正整数制限、customのResourceLocationと任意のNBT payloadも保持する。
payload比較は既存modern NBTの型・NaN・compound規則を使う。
fontとNBT sourceのResourceLocation/元文字列、入れ子clickのgetterも元codecへ照合した。
697入力・583受理値・170,236組のcomponent.equalsが一致する。
`click_constructor_rules/cases/source`に規則・圧縮検査値・元入力hashを分ける。

show_entityはbuiltin entity type、4-word UUID、任意の表示名を共通内部fieldへ読み取る。
UUIDの数値array/listとJDK UUID.fromStringの代替codecを区別し、短いgroup・正符号・
元Character.digit・groupのmaskも保持する。dashlessのauthlib形式は受け付けない。
profileのstrict numeric UUIDとselectorのUUID判定も同じ内部処理を使うが、受付形式は変えない。
表示名はstrict optionalであり、不正なcomponentを省略せず拒否する。
355入力・94受理値のgetter/再帰する表示名・4,465組のcomponent.equalsが一致する。
`entity_tooltip_rules/cases/source`へ元builtin catalog・圧縮検査値・元入力hashを保存する。
表示名の処理制限を伝播し、入れ子item/dialogの未解決依存があれば比較キーを作らない。
この依存伝播のモデル試験は、元codec比較と別に扱う。

再生成は`scripts/export_click_constructors.py`と`scripts/export_entity_tooltips.py`を使う。
既存と同じdownloads/modern-classpath-file/runtime-outputを指定し、JVM512MiB・CPU1で一つずつ実行する。
保存rawとの照合は`--normalize-only --check`。元JAR/JDK・classpath・tool・request・raw・finalを
hashへ束縛し、ゲーム/JDK method bodyやbinaryは配布しない。
これはtooltip表示、world entityの検索、legacyのevent互換、persistent/cache hash、
新しい実ゲーム操作の証拠ではない。一般item/dialogと残る全体統合は継続する。

### enchantment constructorの共通内部field

modernのenchantments/stored_enchantmentsは同じ元constructorを使う。
stream mapの重複referenceは最後の値を残し、その実効mapに対してlevel 0〜255を検査する。
無効levelが後続の有効値で上書きされた場合は受理し、最後の値が無効なら拒否する。
level 0の存在も保持し、未指定へ置き換えない。通常受信とfield取得の両方へこの規則を適用した。
通常受信は全Value treeを作らず、検査に必要なreference/level mapだけを一時保持する。
元patchのbytesと順序、実受信sourceは変えない。

未変更公式codecの92入力・54受理値の実効map、元encoder出力と1,485組のequalsへ一致した。
2種類のcomponent、負数・境界・重複と順序・非canonical varintを含む。
component一値の観測wrapperは末尾byteも拒否し、これは元constructorの範囲検査と区別する。
fieldsのreferenceは当該vanilla oracleのregistry contextに属する。
実接続のregistry解決、全item/prototype比較、persistent/cache hashや在庫操作の許可へ使わない。
新しいlive gameplay検証は含まない。

元map encoderの出力順は再decodeによるmap容量の変化で変わる場合がある。
観測toolは一回の元encodeを記録し、bytesの固定順や繰り返しencodeの不変性を要求しない。
元method/binary/bytecodeは配布せず、規則・圧縮検査値・source hashを
`enchantment_constructor_rules/cases/source`へ分ける。
再生成は`scripts/export_enchantment_constructors.py`を既存と同じ3 path引数で実行する。
JVM512MiB/CPU1で一つずつ実行し、保存rawとの照合は`--normalize-only --check`。

さらに保存済みの全104型・4,134 component値について、入力と元再エンコードのtyped fieldが
一致することを継続検査する。これはfield projectionの試験であり、全component/itemの
意味比較を保証しない。元74 value classの確認ではbundle/container/use-remainder等が
入れ子ItemStack専用の比較を使う。これらのprototype適用・実registry解決と一般data付き操作、
残る全体統合は引き続き対応する。

### bookとenchantability constructorの共通内部field

modernのwritable/written bookを共通内部の`Filtered<T>`へ読み取る。
rawとfilteredの有無・値を別に保持し、同じ値のfilteredが存在する場合も省略しない。
written bookはtitle、author、generation、各text page、resolvedを保持する。
rawとfilteredの両pageからitem/dialog依存を伝播し、未解決の依存があればfield比較を作らない。
比較可能な場合もwhole item/prototype比較やserver cacheの証明とは区別する。

元streamはwritable page 1,024 UTF-16単位・100ページ、written title 32 UTF-16単位を検査する。
written page数にはwritableと同じ100ページ制限を加えない。元codecは101ページも受理する。
written generationは0〜3、enchantableの値は正数でなければ拒否する。
この2つのconstructor制約は通常受信へも適用し、検査に必要な整数だけを取得する。
通常受信で本の全Value treeを作らず、元patch bytesと実受信sourceを保持する。
通常受信での全nested text constructor検証は別の残作業として扱う。

未変更の元codecで118入力・71受理値を実行し、raw/filtered getter・text getter・元encode結果と
2,556組のequalsへ照合した。各scalar・長さ・ページ数境界、空値/filtered有無、Unicode/NUL、
text NBTの単独サロゲート、非canonical varint、各位置の切断と末尾byteを含む。
`book_constructor_rules/cases/source`には規則・圧縮検査値・source hashを保存する。
探索toolの最初の出力は単独サロゲートのUTF-8保存で失敗したため、tool側のJSON transportを
UTF-16単位のASCII escapeへ修正し、元codecで再取得した。元の拒否例として数えない。

再生成は`scripts/export_book_constructors.py`を既存と同じ3 path引数で実行する。
JVM512MiB/CPU1で一つずつ実行し、保存rawの照合は`--normalize-only --check`。
元JAR/mapping/classpath/JDK/tool/request/raw/finalをhashへ束縛する。
legacy book、loreの派生style、実registry解決、全item/prototype比較、persistent/cache、
一般data付き操作と元の全統合範囲は引き続き対応する。

### prototypeとpatchから実効component fieldを組み立てる共通処理

共通内部の`ComponentFields`は、版・component種類・名前を検査したうえで、
itemのprototypeへ追加値を上書きし、明示したcomponentを削除する。
prototypeにないcomponentの削除は実効fieldを変えない。
値はprototypeまたはpatchの元byte列を参照し、元patchや受信sourceを書き換えない。
通常の`ItemStack::properties()`もこの処理から容量・耐久値・componentの存在を取得する。
prototypeの3,490値を一度だけ読み込み、1,505 itemの関連付けを保持する。

未変更の公式ItemStackを使い、全item/prototype検査入力と全104型のcomponent入力から
38,218個の異なるitem入力について、追加・削除後の実効component iteratorを観測した。
検査値のpoolは3,694値。count/空判定、componentの種類・存在、各codecのtyped fieldを照合する。
構造の照合に用いるJSONは検査用であり、native equalsや本番の意味比較には使わない。

検査toolの最初の2回は`getPrototype`と`getComponents`のgetter取り違えを含んでいた。
公式mappingに従い実効値のgetterへ修正し、全入力を実行し直した結果を保存する。
取り違えた取得結果は成功した実効field検証に数えない。
再生成は`scripts/export_effective_item_components.py`を既存と同じ3 path引数で実行する。
元JAR/mapping/classpath/JDK/tool/request/raw/finalをhashへ束縛し、
`effective_item_component_cases/source`へ検査値とsourceを分離する。

この処理はregistry参照を解決しない。prototype内のvanilla IDを実受信registryへ挿入しない。
全item/componentの意味比較、接続・configurationが所有する参照、persistent/cache、
一般data付き操作と元の全統合範囲は引き続き対応する。

### 入れ子itemとbundle constructorの共通内部field

modernのcontainer、charged_projectiles、use_remainder、bundleの入れ子itemを、
版と種類を保持した`RegistryId`、signed i32 count、adapterが所有するpatchの共通内部型へ接続した。
optional item codecはcountが0以下なら空itemになり、airも空として読む。
nonempty codecはどちらも拒否する。元count getterとencoderでこの差を確認した。
containerの末尾空slotを省略せず、listの長さと位置を保持する。元patch bytesと実受信sourceは保持する。

bundleは各itemのprototypeへ追加・削除を適用した重量fieldを読む。
入れ子bundleがあればcontents重量に1/16を加え、なければ非空のbeesは1、それ以外は1/max_stack_size。
各重量にcountを掛け、list順に加算する。bundle/beesの元prototypeはすべて空listであり、
bundle componentの存在自体を保持する。max_stack_sizeの削除は元getterのfallback 1を使う。
容量0や最小i32を一律拒否せず、この分岐を適用した後に元Fractionの算術を検査する。

共通内部Fractionは元factoryの分子・分母表現を保持する。
元equalsは有理数としての等価性とは異なる。符号変更・ゼロ分母、乗算・加算のi32 overflowと
中間計算、元の約分規則を680入力・454受理値・103,285組のequalsへ照合した。
元のApache Commons libraryは公式server bundle内の未変更JARを用いる。

入れ子constructorは425入力・364受理値で、通常受信とfield取得の受理判定、
count/空item/listの元getter、元再encode、bundleの分子・分母へ一致した。
prototype同値patch、容量とcount境界、field順、追加・削除、蜂のみ、入れ子bundle、算術overflow、
切断/末尾byteを含む。通常受信は重量に必要な3種類のfieldだけを保持し、全Value treeを作らない。
既存のdepth/work/byte限界も保つ。このresource limitは元constructorの制約と区別する。

`fraction_constructor_rules/cases/source`と`nested_item_constructor_rules/cases/source`へ
規則・圧縮検査値・source hashを分ける。元の全component.equals 66,430組も後続比較の証拠として
保存するが、Rustのwhole item/component比較が実装済みだとは扱わない。
再生成は`scripts/export_fraction_constructors.py`と`scripts/export_nested_item_constructors.py`を
既存と同じ3 path引数で一つずつ実行する。保存rawの照合は`--normalize-only --check`。
元JAR/mapping/classpath/JDK/tool/request/raw/finalへ束縛し、元method/binary/bytecodeは配布しない。
一般item/prototype意味比較・実registry解決・全component constructor・persistent/cache・
一般data付き操作と元の全統合範囲は引き続き対応する。

元の1.21.11の4種類のcompound component codecについて、End・整数・空compoundの12入力も固定しています。Endと整数は受信時に拒否し、空compoundは受理します。nullable NBTの未検証のconstructorは比較APIで明示的なエラーになります。

Tag参照を含むitemは、比較する両receiptのtag受信sourceが一致することも要求します。再読込をまたぐnamed-holderのobject寿命が未検証のため、この場合はエラーです。Tag参照のないitemにはこの制限を適用しません。
