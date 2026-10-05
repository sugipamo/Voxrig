# Clientの共通化

専用ブランチは`codex/client-api-unification`。developは使用しない。
共通APIの整合性を優先し、依存プロジェクトには移行を要求する。各段階は同じブランチへ積み重ね、
全機能の統合が終わるまではmainへ合流しない。

## 選択と公開入口

通常の利用側は`voxrig::client::prelude::*`を使う。
`ConnectionConfig.version`で接続時に版を選び、接続中は変更しない。
`ConnectionConfig::offline_from_env`は`VOXRIG_MINECRAFT_VERSION`の完全な版名を読み取る。
環境変数はこのsetup補助だけで参照し、既存Clientや別Clientを暗黙に切り替えない。
未設定・不正値・未対応版はネットワーク接続前に拒否する。`latest`も拒否する。
新規版・block/item・properties・物理規則に対応するには、対応adapterとデータを含むVoxrig更新が必要。
未知blockをairや通常のcubeと見なさない。

`Client::survival()`と`Client::creative()`はどちらのadapterでも同じ型のhandleを返す。
この選択はserverのmode変更ではない。各mutationが最新の受信modeと権限を検査する。
Survival handleからcreative writeやcommandは呼べない。Adventure/SpectatorをSurvivalと扱わない。

```rust,no_run
use voxrig::client::prelude::*;
# async fn run() -> Result<()> {
let config = ConnectionConfig::offline_from_env(Server::default(), "AgentOne")?;
let client = Client::connect(config).await?;
client.wait_until_ready().await?;
let survival = client.survival();
survival.select_hotbar(0).await?;
let creative = client.creative();
// 以下はサーバーでcreative modeが受信済みの場合だけ許可される。
creative.set_hotbar(0, Some(("minecraft:stone", 1))).await?;
# client.disconnect().await?;
# Ok(())
# }
```

`ClientLimits`は両版で実装されるtimeout/chunk上限だけを持つ。
以前の`ConnectionOptions`の版固有cache/event/ACK設定をmodern adapterで黙って無視する入口は廃止する。
従来のBot APIでは版固有ConnectionOptionsを引き続き使用できる。

## 型と観測

`Server`、`BlockPos`、`BlockFace`、`Hand`、`Vec3`、`Aabb`の構造を共通側が所有する。
従来importは同じ型のre-export。物理規則はadapterに残る。
特に既存`Aabb::player`は従来の立位寸法であり、任意版・poseの寸法を保証する関数ではない。

`Client::registry()`と`Registry::for_version(version)`は版に束縛した検索入口。
`RegistryId`はversion・item/block-state namespace・native IDを保持する。
別版や別registryのIDを渡しても同じ整数だからと解釈しない。
blockは名前と完全propertiesを要求し、itemはnamespaced名とnative stack上限を使用する。

`Client::server_registry_state()`は実接続から受信したregistry/tagを取得する。
modernのentry/name/native IDはconnectionとconfiguration世代に束縛し、respawnとは寿命を分ける。
legacyは元join codecとtag宣言を保持し、dimension listの個別entryも元順序・元compound bytesで取得する。
固定registry entryは版とregistry名、動的entryは接続とconfigurationの所有情報を保持する。
`find_entry`/`bind_entry`/`entry_name`は両版で同じ検索APIとなる。
modernの動的vanilla IDを未受信のregistryへ補わない。
詳細と残るitem/holder/tagの意味解釈は[受信registry](common-server-registries.md)を参照。

`Client::player_state()`は共通の`PlayerObservation`を返す。
`Client::capture(region)`はplayer/inventory/受信block領域を同じadapter lock境界で取得する。
connection、world generation、receive sequence、cache revisionは別の値で、server tickへ読み替えない。

- `received_pose`は最後の実際の位置packet。現在の予測・送信位置とは別に保持する。
- `ObservedValue.source`は受信ordinal、送信値、client modelを区別する。等しい座標だけで受信と判定しない。
- 在庫slotの`None`は欠測。Emptyは明示的に分かっている空slot。
- 1.16.1は実際のWindow Items/Set Slotから受信表を保持する。クリック後の予測を含み得る従来cacheは`local_cache`へ分離する。
- 1.21.11はcomponent-free stackと、公式codecへ照合した全104型の追加値の境界・全104型の削除patchを元bytesで保持する。複雑な値・入れ子item・registry参照も元bytesで保持し、意味の正規化と実接続の参照解決は別に行う。受信と操作の対応範囲は[共通item data](common-item-data.md)を参照。
- 1.21.11のcursorのordinalはcursorを更新したpacketのもの。無関係なslot更新で新しい受信根拠を作らない。
- `ItemStack::custom_data()`は両版共通のtyped NBT読み取り。元bytesと受信根拠を保持し、native decode・比較・modernの純粋NBT hashを照合した。一般item/prototype統合とdata付き操作は残る。
- `ItemStack::properties()`は現在itemの容量・耐久・stackableを共通fieldへ統合。native default prototypeと追加/削除/受信時補正を使い、signed値と元bytesを保持する。全componentの意味・item比較/hash・slot規則は別途統合する。
- 全104 modern componentのnative値比較・型付きpersistent hash入力・元HashedStack codecをstandalone検査へ保存。共通内部hash計算は8,335 rootで照合しNBT getterへ接続した。完全なcomponent比較や実ServerPlayer cache・data付き操作の統合は続く。
- 全104型のfield grammarを内部の型付きtreeへ接続し、共通propertyの整数読み取りも統合。通常受信はtreeを保持しない。元enum alias・数値型・全kindのunnamed NBTを検査した。constructor/textの完全正規化・live参照・native比較/cache hashは引き続き実装する。
- 内部treeは308のforward codec identityも保持し、Identifierの省略名と不正文字を元両版の274入力へ照合。受信時の検査と元bytes保持を両立する。元textの125候補/3,486比較から、色の別表記とboolean未指定の意味差も記録した。完全なtext/component比較は引き続き統合する。
- 共通内部textモデルに8 contents/11 style fieldを接続。182 wire入力中149 native値のgetterと、依存を含まない9,591比較を照合した。元11,175比較中の残る1,584比較は未解決として保持。完全なconstructor、selector/profile/URI/dialog/item/entity解決、persistent/cacheと操作対応は残る。
- 元fuzzy/strict constructor選択と実mapper順を接続。419入力の354 native成功field・64拒否と41,041比較が一致し、entity拒否1件/21,794比較は未解決。NBT sourceのentity優先、atlas優先、translation fallbackのlenient/hatのstrict、OPEN_FILE禁止を修正。複雑なconstructorの妥当性とそのfallback、persistent/cache/操作は残る。
- 在庫や位置の受信はserver内部状態の独立確認ではない。

`DispatchReceipt`は完全なpacket送信のみを表す。protocol ACKや目的達成ではない。
legacy共通操作はI/O前に未解決markerを保持し、取消・失敗後に自動再送しない。
modernの既存intent・session guardも維持する。保存したreceiptは次の操作許可ではない。

## 実装順序と完了条件

| 段階 | 現状 | 完了条件 |
| --- | --- | --- |
| 1. 設定・基本型・対応情報・registry | 共通fixture・静的検証済み | 両版で同じ共通型。未知版/ID、誤ったnamespaceを拒否。設定を黙って無視しない |
| 2. player/world/inventory共通観測 | 基本capture共通fixture・静的検証済み | 同じcapture境界、受信/予測/欠測を保持。NBTは保持、未対応componentsは欠測として保持 |
| 3. 視点・選択・移動・採掘・設置 | creative基本操作・survival preview/有限dry移動・read-only狙い判定・限定採掘/default cube設置は両版native検証済み。広い移動/採掘/設置条件は残る | 共通request/resultと両版実装、native結果と物理の検証。片版Unsupportedだけでは完了しない |
| 4. container/item data/製作/装備/entity | default player main/hotbar交換は両mode・両版で実装。共通container画面のidentity/内容/slot対応、既に開いたstorage/hotbar交換、opening-bound close、両modeのempty-hand storage openを実装。通常PICKUPで取り出し・split・1個置く・結合・返却も両mode/両版で実装。通常Shift転送とnative default防具への自動装備を両mode/両版で実装。cursor付きcloseのnative版差も両modeで確認し、同値再受信によるlegacy送信前の誤拒否を修正。cursor付き共通closeは実player在庫への返却とstepごとの実受信を組み合わせて実装。modern component全104型の追加値の境界と削除patchを共通観測へ保持。一般UI activation/複雑なitem data/製作/一般装備操作/entityは残る | modern側に受信/クリック/一般item操作を実装。同じ代表workflowと結果検証 |
| 5. context/記録/再構成/scene/復旧 | 残る | 共通型を所有し、legacy側にも版別規則・lifecycleの監査済み実装 |
| 6. UI/特殊window/vehicle/manager | 残る | 各機能の共通操作/観測と両版実装。raw操作自体の版依存は明示的な拡張へ残す |

現在の共通Creativeはdefault hotbar write、look/選択、server許可済みのflight requestと4block以内のstep、
loaded/reachable targetへのcreative break/use-on-blockを実装する。衝突解決や設置成功の保証ではない。
Survivalの追加検査契約は`Client::survival().checked()?`または`Client::checked_survival()`で明示的に選ぶ。
canonical moduleは`client::survival::checked`、旧`checked_survival`は互換alias。
この拡張は現時点で1.21.11専用で、基本共通handleと機能parityを混同しない。
`client::survival::{SurvivalInput, SurvivalControl, PredictedMotionFrame, TerminalClearance}`は
共通側が所有する。従来のmodern/checked importは同じ型をre-exportする。
入力や予測frameの型が共通であることと、各版の実行契約が実装済みであることは別である。
`Survival::preview_path`は両版に実装され、`MotionPreview`を同じ境界で取得する。
版ごとのfloat・trig・collision規則を共有modelへ接続し、legacyの受信poseとlocal physics seedを区別する。
`Survival::start_predicted_path`と`motion_record`も共通化し、有限入力列と診断の型を共通側が所有する。
1.16.1の通常actor dispatchとautonomous physicsがrunと競合しないようにする。
両版で実行中の競合操作を拒否し、位置補正・impulse・変更/失敗を保持する。
詳細と制約は[共通Survivalの移動](common-survival-motion.md)を参照する。
`Survival::target_block`は同じ境界のcaptureと最初のstatic outlineを両版で返す。
`Creative::target_block`も同じ型・shape・版選択を使い、受信creative modeを検査する。
両版とも7種類・全102storage stateのoutline/auxiliaryを公式JARへ照合し、chestのinset等を扱う。
同じnative geometryを両modeの`open_container`へ接続し、完全送信とactual新OPEN/full/cursorを分離する。
modernは実global processing ACKも保持し、legacyに相当値を発明しない。
OPENにはblock座標がなく、受信したmatching screenをtarget由来の因果証明にはしない。
詳細・限定範囲・取消は[共通container open](common-container-open.md)を参照する。
視線/traversal kernelを共有し、shapeは各版で検証したデータから選ぶ。採掘・設置の実行許可ではない。
範囲と独立native oracleは[共通Survivalのブロック狙い判定](common-survival-targeting.md)を参照する。

`Survival::start_mining` / `finish_mining` / `abort_mining` / `mining_record`も両版に実装する。
受信済み空手でのdirt/stoneに限定し、before-I/O intent、native protocolの違い、fresh target receipt、
取消/競合の履歴を保持する。airやABORTで元接続のmutationを解放しない。
詳細は[共通Survivalの採掘](common-survival-mining.md)を参照する。

`Survival::place_cube` / `placement_record`も両版で同じ公開型を返す。
最初のnative outline・受信済みdefault material・空の隣接targetを検査し、before-I/O intentと
新しい対象block/1個の材料消費を保持する。modernだけが持つprocessing sequence/ACKを区別する。
未解決の間は再送/競合を拒否し、完了後の新しい操作は現在の条件を再検証する。
legacyの特殊なraw inventory更新も公式実装へ照合し、hotbar/armor/offhandの画面番号へ正しく変換する。
詳細は[共通Survivalの設置](common-survival-placement.md)を参照する。

両modeの`swap_hotbar`と`inventory_swap_record`は、解決できるNBT/components付きplayer stackも共通のopaque attempt/recordで交換する。
元の両slotの実受信をI/O前に保持し、両方の新しいdestination、legacyの比較応答、modernの画面revisionを区別する。
legacyはnativeの更新抑止を避けるfull resyncを一度のクリックで要求し、negative比較応答をrollbackへ読み替えない。
取消・途中の競合・閉じた接続の診断を保持する。一般containerの実装完了にはしない。
詳細は[共通在庫交換](common-inventory-swaps.md)を参照する。
close後の通常在庫交換も両版に実装する。received active windowを0へ変更せず、
local player UIを`SubmittedClose`として別に保持し、modernのactual player-screen-zero revisionを使う。
再OPEN/respawn/reconfigurationで元のbasisを失効させる。詳細は[共通プレイヤー画面](common-player-screen.md)を参照する。

通常slotのnative条件をSWAPの送信前に検査し、shulker boxへのshulker box収納をowner/packet前に拒否する。
legacyのitem容量も元JARへ揃え、warped_fungus_on_a_stickの64→1を元upstream dataを変えずに修正した。
PICKUP/split/返却は元menu 18,432ケースずつ、default cursor比較は元codec 120件ずつを照合したが、
共通`click_inventory` / `inventory_click_record`を両mode/両版に実装する。予測と実受信を分離し、
元opening、default predecessor、実source/cursor更新とlegacy replyを検査する。Shift転送は[共通転送](common-inventory-transfers.md)で実装し、一般item dataは残る。
[検証範囲とslot条件](common-inventory-clicks.md)、[更新後のnative回帰](common-client-native-validation.md)を参照する。
送信前の同値再受信と、cursor付きcloseの版差は[追加native調査](common-cursor-close-audit.md)を参照する。

`Client::capabilities()` / `Capabilities::for_version`は共通面の実装状況を返す。
NotImplementedはVoxrig側の不足であって、ゲームに存在しないという意味ではない。
既存Botにあるcontainer等も共通入口が未実装なら共通capabilityではNotImplementedになる。
対応情報は現在の権限・鮮度・実行許可ではない。

## 検証

prototype・patchの実効component適用は共通内部処理へまとめた。
通常のitem property取得もこの処理を使う。公式ItemStackの追加・削除後のiteratorと、
38,218入力・全104型・3,694値の構造を照合する。
値の元byte列と受信sourceを保持し、native equals・live registry・server cacheや
一般data付き操作の完成とは区別する。詳しい契約は`common-item-data.md`を参照する。

同じconsumer scenarioを両adapterのnative packet fixtureへ通す。版別のbootstrapはfixtureが担当し、
consumer部分にMinecraftVersion分岐を置かない。mode違反、範囲外pitch/flight、受信されていない在庫、
取消後の再送、cache予測混入、cursor根拠の取り違え、誤ったregistry identityを検査する。
1.16.1の送信packet ID/field形式は[minecraft-dataの1.16.1定義](https://github.com/PrismarineJS/minecraft-data/blob/master/data/pc/1.16.1/protocol.json)とも照合する。
creative slotは0x27で、0x28は別操作。受信/送信fixtureだけが同じ誤ったIDを使って通る試験にしない。
packet fixture試験は実ゲームの独立した結果検証ではない。後続のmovement/container等では
各版のnative serverと独立した結果観測も必要で、fixture成功だけで実ゲーム互換を宣言しない。

ビルド・テスト・native serverは一つずつ実行する。Cargoは`-j1`、testは`--test-threads=1`。
利用例のsetupだけの確認は次で行える。

```bash
VOXRIG_MINECRAFT_VERSION=1.16.1 cargo run --locked -j1 --example common_client -- --check
VOXRIG_MINECRAFT_VERSION=1.21.11 cargo run --locked -j1 --example common_client -- --check
```

接続・観測だけのsmokeを行う場合は`--check`を外す。HOST/PORT/USERNAMEも環境変数で指定できる。
mode変更、建築、採掘等を勝手に実行するexampleにはしない。

## この段階の検証結果

main `af91cad`を基準に、共通基盤・capture・mode handle・creative基本操作を追加した。
共通read-only preview・狙い判定・限定採掘/設置・在庫交換と版別native model照合を追加し、全targetのunit/fixture試験は420件成功、専用native環境7件と性能1件はignored。
doc test 5件、fmt、警告をerrorにするClippy/Rustdoc、Rust 1.85の全target check、
native検証スクリプト・共通移動/狙い判定/設置/在庫交換を含めた374ファイルのpackage buildが成功。
続いて同じ共通API consumerを公式vanilla両版で実行し、creativeの在庫・選択・flight・除去・設置・
survival切替後のcreative write拒否をサーバーRCONで確認した。
両版で同じ35tickのjump/歩行previewも取得し、preview前後で実際の位置が変わらないことをRCONで確認した。
有限jump/歩行も同じ共通consumerから実行し、RCONで途中の上昇と予測終点との一致を検証した。
競合操作、I/O前のintent、待機取消、地形変更・impulse・送信失敗、native-onlyへの切替後の診断保持も検査した。
狙い判定も共通consumerで実行し、選んだstoneの実際のstateとquery前後の位置不変をRCONで確認した。
公式JARのlegacy 72 rotation/585 ray/13 outline状態と、modernの2,304 rotation/25,394 rayも共有kernelへ照合した。
両サーバーも正常終了した。限定survival採掘も新しい接続で実行し、START/FINISHの実送信・fresh air受信とRCONの対象air・位置不変を検証した。
続く新しい接続でdefault dirtを設置し、fresh target/material受信とRCONのdirt・材料3→2・位置不変を照合した。
さらに新しい接続でsurvivalのoccupied player swapとcreativeのempty-destination swapを実行し、
両slotの実受信とRCONのslot/item/count、位置不変を照合した。legacyのnative codecでも3fixtureを確認した。
限定範囲外の条件と後続の機能統合は残る。
再実行方法・証拠・範囲は[共通Clientのnative検証](common-client-native-validation.md)に記録する。
これは全段階の機能parity完了の記録ではない。

### cursor付きcloseのAPI変更

`close_container`は十分な既知player main/hotbar容量があれば、既知cursorを実在庫へ返してから閉じる。
`ContainerCloseStage::ReturningCursor`、`ContainerCloseRecord::{return_plan, return_steps}`と
`InventoryClickId::close()`を追加した。stageを全分岐している利用側は新variantへ対応する。
両版ともcallerの待機取消でowned返却を止めず、同じScreenIdの再呼び出しは拒否する。
進捗は`container_close_record`で確認し、RequiresInspectionの操作は自動再試行しない。
通常click履歴とclose内部stepは別に保持する。詳細は[共通close](common-container-close.md)を参照。

### profile fieldの内部共通化

modernのprofile componentとtextのplayer objectは共通の内部constructor fieldへ読み取る。
NBT/通信の異なる名前制限、full/partialの種類、propertiesの順番・重複・署名、skin patchを保持し、
公式codecの324入力・253受理値・32,131組の比較へ照合した。公開操作の拡大やskin解決は行わず、
一般item data・persistent/cache・残る全機能の統合は継続する。詳細は[共通item data](common-item-data.md)を参照。

### selector fieldの内部共通化

modern textのselectorとscoreを共通内部constructorへ接続した。元21 options、raw UTF-16と
消費cursor、scoreのselector/literal分岐、SNBT predicate grammarを検査し、1,955入力・998受理値・
498,501組の元比較に一致した。text coreの比較は10,585組、拡張constructorの比較は61,425組へ広がった。
これらは重複するため加算しない。entity query実行、URI/dialog/item/entityの完全な構築、
一般item操作と残る全体統合は継続する。詳細は[共通item data](common-item-data.md)を参照。

### URL fieldの内部共通化

modern open_url clickを共通内部URI constructorへ接続した。raw UTF-16と元URI getter、
opaque/hierarchical・server/registry authority・percent case等の元比較を保持する。
1,810入力・1,089受理値・593,505組の元比較へ照合し、現在のtext core比較は10,731組、
拡張constructor比較は61,776組になった。通常受信はbytesを保持し、URLを開く処理は行わない。
残るfont/click/hover/dialog/item/entity constructor、一般item意味比較と操作の統合は継続する。
詳細は[共通item data](common-item-data.md)を参照。

### 入れ子itemとbundle constructorの内部共通化

modernの入れ子itemを共通内部型へ接続し、optional/nonemptyの空itemの差とcontainerの空slotを保持する。
bundleはprototypeへの追加・削除、入れ子bundle・蜂・容量の分岐とchecked Fraction計算を適用する。
通常受信は全Value treeを作らず、必要なfieldだけを読む。
Fractionの680入力・103,285比較と、入れ子constructorの425入力・364受理値の受信/getter/重量へ一致した。
全item/prototype比較とlive context、一般data付き操作、元の全統合範囲は継続する。
詳細は[共通item data](common-item-data.md)を参照。

### book constructorの内部共通化

modernのwritable/written bookを共通内部fieldへ接続し、rawとfilteredの値・有無を保持する。
written pageの両方から未解決のitem/dialog依存を伝播する。
enchantableの正数制約とwritten generation 0〜3は通常受信へも適用する。
118入力・71受理値・2,556組の元比較に一致する。writableの100ページ制限をwrittenへ流用しない。
legacy book・lore・一般item/prototypeとdata付き操作、元の全統合範囲は継続する。
詳細は[共通item data](common-item-data.md)を参照。

### clickとentity tooltipの内部共通化

modernのrun/suggest commandを元CHAT_STRINGへ合わせ、clipboardと区別した。
page/custom payload/font/NBT sourceを含む697入力・583受理値・170,236比較に一致する。
entity tooltipのbuiltin type・UUID・strict optional nameは355入力・94受理値・4,465比較に一致し、
profile/selectorと共通のUUID処理を使う。表示名内のitem/dialog依存と処理制限は伝播する。
既存拡張text検査の65拒否すべてが一致する。これは内部constructor統合であり、
一般item操作・live cache・crafting/entity/context/recovery等の全体統合は継続する。
詳細は[共通item data](common-item-data.md)を参照。

### enchantment constructorの内部共通化

modernのenchantments/stored_enchantmentsを共通内部mapへ接続した。
重複keyを最後の値へまとめてからlevel 0〜255を検査し、通常受信でも同じ規則を使う。
92入力・54受理値・1,485元比較に一致し、全104型・4,134値のtyped field往復も継続検査する。
実registry binding・全item/prototype比較・一般data付き在庫操作と元の全統合範囲は継続する。
詳細は[共通item data](common-item-data.md)を参照。

### 受信itemとregistry所有情報の同時取得

`Client::received_inventory()`は両adapterで実受信slot/cursorとregistryを同じロック境界から取得する。
公開constructorのない`ReceivedInventory` / `ReceivedSlot` / `ReceivedItem`は元item bytes・packet ordinal・
接続/world/configurationを読み取り専用で保持する。legacyのlocal cache/physics取得を必要とせず、
JSONに大きいregistry payloadをslotごとに複製しない。
同じitem bytesでも再設定でIDの所有者が変わるpacket検査と、TCP再接続・legacy cache予測の
検査を行う。一般component/item意味比較と元の全統合範囲は継続する。
詳細は[共通item data](common-item-data.md)を参照。

### 受信itemの型付き比較

両版の`ReceivedItem::native_equivalent`は元wire/JSON/CRCの比較を使わず、
legacy NBT constructorとmodern prototype/patchの実効値へ接続する。
実受信registry ownerへ束縛したfield比較と、同じdecoded receiptの位置を保持する。
保存済み元JVM corpusのlegacy4,805/modern13,853 item、modern全104型/4,134 component、
24,789 prototype操作・66,430 nested比較へ一致する。新しいholder/tag observerは
lookup/streamのobject共有と未宣言tag拒否を別々に確認する。
任意のtext内item/dialog・tag再読込のlookup寿命・server cache/slot規則と一般操作、
元のmovement/context/記録/再構成/復旧/UI/vehicle/manager統合は継続する。
詳細は[共通item data](common-item-data.md)を参照。

Text内のitem hoverとdialog参照も受信所有情報付き比較へ接続した。
元NBTからconstructorを再構成し、入れ子の既知の拒否をfuzzy/strict/lenientへ伝播する。
未実装constructorをfallbackで隠さない。入れ子textと本のpageにも同じ処理を適用する。
507入力の元getter/canonical/equalsを保持し、413値・85,491比較と90拒否が一致する。
任意persistent componentとinline dialog、一般item操作/cacheや後続の広い統合範囲は引き続き実装する。

在庫SWAPのdata付きstack受付と結果比較を両版へ接続した。legacy比較packetにNBTを保持し、
modernは元prototype/patchの実効容量を使う。保持したregistryとtyped native fieldでfreshな両destinationを
照合し、configuration/tag sourceの変更や意味解決の失敗をinspectionとして残す。
通常PICKUPも受信NBT/components付きstackのsplit・1個置く・全返却へ接続した。
modernは送信revisionを明示して完全再同期を要求し、実source/cursorのfresh受信で結果を検査する。
通常QUICK_MOVEも受信NBT/components付きstackの実効容量・stackable/data同値判定へ接続した。
全changed slotsのfresh実受信と、保持したregistryの意味で完了を検査する。
modernの変更したequippable routingとarmorのallowed_entitiesを実効componentから判定する。
直接list・受信named tag、装備占有時の振り分け、armorへの1個と残りのmain/hotbar転送を区別する。
通常itemのdata付きcursor closeも、registry付き計画・step準備・実source/cursor受信へ接続した。
各stepでdataの正規化を許容し、実Emptyを確認してからCLOSEを一度送る。
data付き装備済みarmorの取り出しも両版に接続した。survivalの束縛制限とcreativeの取り出しを区別し、
modernは受信enchantmentの制限効果を解決する。
非空cursor QUICK_MOVE、特殊item override・製作/一般装備や
後続の統合範囲は継続する。
