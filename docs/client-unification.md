# Client共通化の設計とロードマップ

専用ブランチは`codex/client-api-unification`。developは使用しない。
共通APIの整合性を優先し、依存プロジェクトには移行を要求する。各段階は同じブランチへ積み重ね、
全機能の統合が終わるまではmainへ合流しない。

2026-10-06に進め方を見直し、利用者の指示で新しいゴールを作成して再開した。下記のロードマップは、
既存成果を使って全体を通す順序と、後から対応範囲を広げる順序を分けたもの。
最終目標は、接続時の版選択を除いて同じClient APIで利用できること。
新しい版・未知block/itemへの対応にはVoxrig更新を要求する。

## 現在の区切り

A0〜A5の代表操作は両版で初回貫通済み。A6は資料・共通consumerを準備済みで、
非公開の利用側から固定commitでの移行結果を受け取る段階にある。
全機能の統合完了とは分ける。B4の結果転送に続き、B6の有限乗車入力を一連の操作として閉じた。
vehicleの受信motion観測と、下車後の通常地上停止→新しい歩行→収納も両版・両modeで接続した。
boss barに加え、title／action bar／CLEAR／RESETとworld border更新も両modeの実Clientで接続した。
tab header／footerは原codecとadapter適用に加え、新版再設定試験で実配信も確認した。
teamsとplayer一覧も実受信の共通入口へ接続した。
次は特殊vehicle／window／managerと残るUIを
利用操作の単位で進め、B3〜B5の残機能も継続する。
B5の新版再configurationではUI／player一覧のcache寿命・再登録と新しい収納操作への継続を接続した。
通常死亡後の明示的respawn→新world観測→接地→新しい収納操作も両版・両modeで接続した。
次はchunk欠測・再読込と再接続を一連で通す。次元移動・Hardcore／credits等の広いB5は残る。
操作に不要なconstructor比較の拡大を先行させない。
実サーバーや重い検査は一つずつ実行する。

## 再開後の進捗

- A0完了: mainのPR #6・#7を専用ブランチへ取り込んだ（`ab3b5aa`）。
  共通のcomponent付き在庫も診断recordへ元bytes・削除patch・版を保持し、
  保存データから検証済みIDや操作guardを復元しない。
  後続のmain `b38e8b4`（PR #10の旧版generation revocation）も取り込んだ（`f05cff2`）。
  nativeの緊急遮断を維持し、Client共通の緊急遮断入口もB5で接続した。
- A1完了: 同じconsumerを使い、各modeで接続を変えずに観測・地上移動・収納・通常製作・
  設置・切断を両版の公式vanillaで確認した。収納後のno-echo closeから設置へ進む契約を接続し、
  Creative handleにも有限地上歩行を実装した。予測位置と受信pose、close送信と受信画面は分離する。
- A2完了: 同じClientでbootsの装備転送、sheepへの一回の攻撃、
  villagerへの一回のinteractionと実merchant OPENを両版・両modeで確認した。
  削除済み・再利用されたentity IDを拒否し、spawn座標を現在位置とは扱わない。
  単体681件、公開API2件、doctest24件が成功した。詳細は[基本装備とentity操作](common-client-entities.md)。
- A3完了: 共通の同一profile復旧を実装し、両版で採掘完了後・早いFINISHの未解決状態から
  明示的close、fresh admission、新しい設置まで確認した。旧接続・旧IDは解放せず、
  取消したloginも二度目を拒否する。元のdelayed miningが予定時間後にも継続しないことを確認した。
  詳細は[共通の採掘復旧](common-mining-recovery.md)。
- A4完了: 接続開始からの実受信packet記録、各版decoderによる読み取り専用のplayer／inventory／
  block再生、共通の限定scene captureとnative予測を両版で確認した。元の全packetをproxyで照合し、
  NBT/component bytesと受信ordinalを保持する。地形変更・切断後もsceneは不変で、保存値から
  操作ID・Clientを復元しない。詳細は[記録・再生・scene](common-recording-scenes.md)。
- A5完了（代表操作）: 共通scoreboard観測とClientManagerを実装し、両版で2つの接続の値更新・owner reset・
  objective削除・manager終了を確認した。生成ごとの版／registry分離と接続取消は軽量TCP試験で確認する。
  かまどのconstructor由来のslot観測・PICKUP・燃料tag検査・開閉も接続した。
  両版・両modeの精錬workflowを検証した。共通`vehicle_state`へ実passenger関係の観測を接続し、
  同じClientから一回の乗車interaction→owned下車要求→実除外受信→明示的neutral→切断を
  両版・両modeで検証した。独立したnative RootVehicleと元packetのfields／ordinalも照合した。
  詳細は[基本UIとmanager](common-ui-manager.md)と[共通かまど操作](common-furnaces.md)、[共通乗車関係](common-vehicles.md)。
- A6の共通consumer・対応一覧・移行資料は準備済み。非公開利用側の固定commit検証は結果待ち。
  Bの広い残機能は引き続き必須作業で、全統合完了とは扱わない。
  初回貫通時はownedレシピブック配置を退避し、現在はB4として復元・実装している。
  利用側へ返してもらう内容は[A6の確認手順](client-api-consumer-validation.md)にまとめる。
- B3の一部完了: 乾いた登録stairs/slabを両版の有限地上歩行・立位検査・Survival sceneへ接続した。
  同じconsumerで段差を上り、同じClientで収納の開閉と切断まで両版・両modeで確認した。
  公式combined VoxelShapeの境界を保持し、実受信propertyから形状を選ぶ。
  waterlogged／未知形状は拒否し、道具・effect・姿勢・広い採掘／設置は残る。
  詳細は[共通dry terrain](common-dry-terrain.md)。A6の利用側評価とBの全残機能を完了扱いにしない。

- B3の一部完了: 地上移動と収納済みのClientからCreative飛行を要求し、
  3つの短い飛行位置と飛行解除まで両版の公式vanillaで確認した。
  flight commandをClient所有の一回送信として保持し、古い地上終点は診断履歴へ退避する。
  続いて`creative().land()`を接続し、同じClientで床へ戻る→明示的解除／neutral→
  2つのreleased ground model ticks→新しい27tick地上移動→収納の再開閉まで両版で確認した。
  zero controller seedは宣言値として保持し、実受信pose／velocity／abilitiesを上書きしない。
  正常なstanding・通常attribute・effect／未解決impulseなし・既知dry supportに限定する。
  飛行物理全体や別姿勢／effectへの拡張は残る。
  詳細は[共通Creative飛行](common-creative-flight.md)。

- B3の一部完了: 通常道具と耐久値だけを持つitem dataを共通Survival採掘へ接続した。
  同じconsumerでpickaxe／stone、shovel／dirt、axe／planks、wooden pickaxe／double slabを採掘し、
  明示的な同一profile復旧→新しい設置まで両版で確認する。
  元native getterに基づく`MiningEstimate`はローカルの目安として保持し、実受信結果へ変換しない。
  道具交換の最初の受信は保持し、正常なtarget airの後に届く耐久値更新で除去履歴を消さない。
  元接続を次のmutationへ解放せず、欠測tag／対象のtag変更を拒否する。
  enchantment／custom tool／effect／別姿勢、広い移動・採掘・設置は引き続きB3へ残す。
  詳細は[共通採掘](common-survival-mining.md)。B4〜B6とA6も継続する。

- B5の一部完了: `Client::revoke_connection()`を両版へ接続した。
  観測capture・writer・通常のdisconnect cleanupを待たず、元transportと全cloneを不可逆に遮断する。
  旧版のnative generation fenceと、新版のpartial-write uncertaintyを維持する。
  同じconsumerで実接続→通常選択→遮断→元Client／cloneの追加操作拒否を両版で確認し、
  両handleを保持したままnative playerの不在とproxy EOFを独立観測した。
  ロック待ち・別OS thread・32送信待ち・partial prefixと別接続の分離は軽量試験で確認する。
  広いcontext／記録／再構成／復旧と各履歴取得経路は継続する。
  詳細は[共通緊急遮断](common-connection-revocation.md)。B3／B4／B6とA6も残る。

- main `686f414`（PR #8・#9・#11・#12）との整合を更新した。
  旧版のlength-prefixed light arrays、地形更新時のlight鮮度無効化、opt-in protocol診断、
  modern detached movement／removal successorを維持する。
  checked APIのexport競合は共通motion型を維持し、新しいtransition型を追加して解決する。
  これらの取り込みはB5全体の共通化完了を意味しない。

- B4の一部完了: sealed `recipe_placement_plan`から一回のowned配置を送信し、
  実入力と在庫の保存を確認した後、明示的結果取得→格納→空grid／cursor→製作台close→切断を通した。
  両版・両mode・player／table・Next／Maximumの16ケースでnative在庫と位置を独立確認する。
  材料の保存が確認できるまで後続mutationを許可せず、古いplanの再送を拒否する。
  材料不足ghostと非空cursorへの結果全体の結合も接続した。
  結果のnative QUICK_MOVEも接続し、両版・両mode・player／table・Next／Maximumの16ケースで、
  一回の送信→実grid／主在庫の増加と変わらない実cursor→close→選択→切断を確認した。
  製作総数や後続の部分移動・dropは予測しない。
  table入力SWAP・QUICK_MOVEと広い製作・一般装備・entity・dataはB4へ残す。
  隣接する製作台がdry collision範囲に入るとlookを拒否する制限はB3へ残す。
  詳細は[共通レシピ](common-recipes.md)。A6と全体goalは継続する。

- B6の一部完了: 共通`start_vehicle_control`へ有限digital入力を接続した。
  同じClientで短い地上移動→実乗車→12入力＋2neutral→実下車＋neutral→切断を、
  両版・両modeの4件で確認した。入力送信と実車両移動／停止の意味を区別し、
  完了済み地上runは元履歴を保持して退役させる。途中送信や乗車変更は解放・再送しない。
  受信motionは次の区切りで接続した。metadata・paddle・広いphysics・下車後の地上継続、他UI/window/managerはB6へ残す。
  詳細は[共通乗車入力](common-vehicles.md#有限の乗車入力)と[検証記録](common-client-native-validation.md#b6-finite-mounted-input-and-original-minecart-response)。

- B3／B6の前提: 車両の近くで歩けるように、四種類の乾いたrailの元collision／outlineを
  共通地形へ接続した。両版の46状態を完全propertiesで選び、railの下の既知床で立位を検査する。
  同じ車両fixtureへの前進をnativeで確認し、下車後の地上継続は次の実装へ残す。
  詳細は[共通dry terrain](common-dry-terrain.md#車両付近の乾いたrail)。

- B6の一部完了: `titles()`／`tab_list()`／`world_border()`を両adapterへ接続した。
  title命令とborderのSET→LERPを両modeの同じClientで観測し、元fieldのordinalへ独立照合する。
  CLEAR／RESETでaction barを消さず、別worldのborderと未受信defaultを合成しない。
  tabはheader／footerのみで、teams／player一覧は下記の別入口へ接続する。広いB6は残る。
  詳細は[共通表示情報](common-ui-display.md)。

- B6の一部完了: `teams()`／`player_list()`を両adapterへ接続した。
  同じClientでteamの宣言→metadata／所属移動→LEAVE／REMOVE、peer mode更新と個別切断による
  実roster REMOVE→残るmanager終了を確認する。offline holderとonline profileを区別し、
  元の全field encodingと受信ordinalを保持する。旧版のNOT_SET、版ごとのlisted／chat／order／hatも
  defaultへ読み替えない。特殊window／vehicle／広いmanagerとB3〜B5、非公開A6は継続する。
  詳細は[共通teamとplayer一覧](common-teams-player-list.md)。

- B5の一部完了: 新版の実START_CONFIGURATIONでglobal UI／profile登録をresetし、
  別fieldの`context_reset_sequence`に実ordinalを保持する。原GUIが残すaction barは元originで保持。
  同じ2接続のSurvival／Creativeで再設定中の欠測・旧screen拒否→実registry再受信／play→
  旧screen再拒否→新しいチェスト取得／格納／close→manager終了を接続した。
  [共通UI context](common-ui-context.md)。広いB5とB3／B4／B6・非公開A6は継続する。

- B5の一部完了: `Client::respawn()`／同期`respawn_record()`を両adapterへ接続した。
  現在worldの実death healthから一回だけ要求し、実RESPAWN・fresh pose／正のhealthを待つ。
  新版の空中開始はowned実respawnとfreshゼロvelocityを条件にreleased入力だけを許可する。
  両modeで4released ticks→旧screen拒否→新チェスト取得／格納／close→manager終了まで確認した。
  実受信spawn poseと予測床終点・送信済み位置を分け、global UI／registryと別Clientのworldを保つ。
  [共通respawn](common-respawn.md)。chunk欠測／再読込・再接続と広いB5、B3／B4／B6・非公開A6は継続する。

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

## ロードマップと現在地

見直し時の調査基準はPR #4の実装commit `24fef69`、当時のmainはPR #6・#7を含む
`344018c`。再開後の進捗は上の一覧に記録する。後続のmain `b38e8b4`（PR #10）も
専用ブランチへ取り込み、その後`686f414`の照明・診断・仮想transitionも取り込んだ。
mainのnative機能とClient共通化の完了を区別する。
過去の[派生版rollout](client-rollout-roadmap.md)と
[1.16.1 API拡張](headless-api-roadmap.md)は、この共通化の完了表ではない。

| 段階 | 対象 | PR #4の見直し時点 | 全体を通す初回の到達点 |
| --- | --- | --- | --- |
| 1 | 接続設定・基本型・対応情報・registry | 共通入口と版選択の基盤は実装・検証済み | 同じ利用コードで接続でき、版・ID・registryの混同を拒否する |
| 2 | player・world・inventoryの観測 | 基本captureと受信／予測／欠測の区別は実装・検証済み | 次の操作へ必要な実受信情報を同じ境界で取得できる |
| 3 | 視点・選択・移動・採掘・設置 | 両版の限定条件で動作・実サーバー検証済み。共通fresh復旧も接続済み。広い地形・道具条件はB3へ残る | dry環境の基本操作から、明示的な復旧を挟んで次の操作へ進める |
| 4 | 在庫・container・製作・装備・entity | 通常在庫・開閉・製作入力・空/互換cursorへの結果全体取得と通常owned配置・材料不足ghostは実装済み。基本装備／entityは初回貫通済み。広い対応はB4 | 収納と通常の1回製作、基本装備、代表的なentity操作を同じAPIで通す |
| 5 | context・記録・再構成・scene・復旧 | 元packet記録・選択観測の再生・限定scene・明示的fresh復旧・共通遮断は実装済み。広いcontextと再構成はB5 | 受信記録の読み取り専用再生、限定scene、明示的な復旧から操作を継続できる |
| 6 | UI・特殊window・vehicle・manager | scoreboard／かまど／実乗車関係・下車／managerは初回貫通済み。広い対応はB6 | 代表的なUI・特殊window・乗車状態／下車・複数Client管理を両版で通す |

**見直し前の作業は第4段階の「レシピブック配置」だった。現在はA0〜A5の初回貫通後、B4として通常配置を接続した。第4段階全体の完了ではない。**

- 公開済み: 材料判定、盤面の配置幾何、返却計画、`recipe_placement_plan`。
  配置planは読み取り専用で、配置packetを送信しない。
- 中断時の作業: planからのowned送信、実入力と在庫の結果保持、取消・競合の扱い。
  初回貫通中は退避した。現在はB4で復元し、通常Next／Maximumのowned送信と実保存、結果のnative QUICK_MOVEまで接続した。
- 既に使える製作経路: `click_inventory`で通常の入力を置き、
  `take_crafting_result`で空または互換の実受信cursorへ結果全体を取得する。必要なrecipeと入力手順は利用側が選ぶ。

レシピブック送信は有用な機能であり、最終的な残作業に含む。しかし、
**既存の通常クリック経路で基本製作を通せるため、初回の全体貫通を待たせる必須条件にしない。**
この区別が、これまでの「第4段階の前提を精密化し続ける」進め方との主な変更である。

### A. まず全体を通す

以下は合意した実装順序。代表例が動いた段階を「初回貫通」と呼び、
全機能の統合完了やmain合流の許可とは分ける。各項目の未実装部分は実装する。
型のre-export、版固有APIへの入口、`Unsupported`だけでは合格にしない。

| 順序 | 利用者が完了できること | 合格条件 |
| --- | --- | --- |
| A0 | 機能の対応範囲を判断する | mainのPR #6・#7との整合を確認し、共通／片版専用／未実装の一覧と代表シナリオを固定する |
| A1 | 接続→観測→移動→収納→通常製作→設置→切断 | 既存APIを組み合わせた同じconsumerが両版で一連の操作を通す。Survivalはdry環境・通常item・1回製作、Creativeは受信modeに合う基本操作を使用。結果・残った在庫・cursor・画面を確認する |
| A2 | 基本装備と代表的なentity操作を行う | 既存の装備転送をシナリオに組み込み、実在する同じentity instanceへの基本interactionを両版で実装・確認する。装備選択や戦術は利用側に残す |
| A3 | 採掘後に明示的に復旧し、次の操作へ進む | 両版で共通のlifecycleと新しい受信基準を確認する。現状の採掘はair受信だけで次のmutationを許可しないため、復旧を実装するまで連続採掘や採掘→設置を成功扱いにしない |
| A4 | 記録した場面を再確認し、限定sceneを検討する | 同じ共通APIで受信記録を各版decoderへ読み取り専用再生し、選んだplayer／block／inventory観測を照合する。限定sceneのcaptureと予測も両版へ接続し、保存値から実行許可を再生成しない |
| A5 | 基本UI・特殊window・乗車状態・複数Clientを扱う | UIはscoreboard観測、特殊windowはかまどの基本slot操作、vehicleは実乗車状態の観測と明示的な下車、managerは生成・取得・終了と版／registryの分離を代表例にする。両版で実操作・実受信を通す |
| A6 | 全体を利用側から評価する | 共通consumer、対応機能一覧、移行資料を揃えて固定commitを提示する。依存プロジェクトは非公開のまま移行・検証し、不足を返す |

A1では採掘からの継続をまだ組み込まず、A3で追加する。初期資材やworld配置は
試験fixtureとして準備できる。自動採集・経路選択・製作するものの決定はClientへ追加しない。
原則として版の変更はsetupに限定し、consumerへ版別操作分岐を追加しない。
modeの違いと宣言した対応条件は、利用者が判断できる形で残す。

A5の代表例は初回貫通用であり、すべての特殊windowやentity／vehicle機能の統合を意味しない。
混在版のmanager分離は軽いfixtureで確認し、実サーバーは各版を順番に実行する。
全体を通した結果、代表例では足りない利用上の要件をBへ具体的なケースとして追加する。

### A0で固定する範囲

| API／機能 | 1.16.1 | 1.21.11 | 初回の扱い |
| --- | --- | --- | --- |
| 接続・registry・player/world/inventory観測 | 共通 | 共通 | A1で同じconsumerの受信基準に使う |
| 視点・選択・乾いた地形での有限移動 | 共通・限定条件 | 共通・限定条件 | 両handleの有限地上歩行。Creativeでも飛行中は拒否する |
| storage開閉・通常クリック・転送・空/互換cursorへの製作結果取得 | 共通・限定条件 | 共通・限定条件 | 一つの接続で収納の後に通常製作へ進む |
| passive cube設置・通常道具の既知dry採掘 | 共通・限定条件 | 共通・限定条件 | A1は設置。採掘後の共通復旧はA3 |
| 装備への通常転送 | 共通・限定条件 | 共通・限定条件 | A2の代表シナリオに組み込む |
| 限定captured scene・packet診断recordと選択観測の再生 | 共通・限定条件 | 共通・限定条件 | A4で同じconsumerを検証。live native予測と一致し、source更新・切断後も不変 |
| checked拡張・assumed scene・仮想編集/連鎖・広い再構成 | 共通契約は未完了 | 版固有拡張 | PR #7の機能を維持し、Bで拡大。re-exportだけで両版対応としない |
| entity spawn寿命・一回のINTERACT／ATTACK | 共通・限定条件 | 共通・限定条件 | A2で装備→攻撃→削除拒否→村人interactionを確認。現在のmotion/metadata・INTERACT_ATはB |
| scoreboard観測・ClientManager生成／取得／終了 | 共通・限定条件 | 共通・限定条件 | A5の一部を実装・検証。managerのmixed-version分離は軽量fixtureで確認 |
| かまどslot観測・通常PICKUP・開閉 | 共通・限定条件 | 共通・限定条件 | A5の基本かまど精錬flow。溶鉱炉／燻製器はconstructor/slot規則の確認で、特殊レシピのlive検証は残る |
| own-player乗車関係・明示的下車 | 共通・限定条件 | 共通・限定条件 | 実passenger list、owned一回送信→実除外→neutral。両版・両modeのnativeで確認。通常地上への明示的継続もB6で接続済み |
| 有限digital乗車入力 | 共通・限定条件 | 共通・限定条件 | 受信MountIdへ有限入力＋最終neutral。受信位置は共通entity_motionで観測。通常下車後の地上継続は接続済み。特殊補間・physics・paddleはB6 |
| title／action bar・tab header/footer・border観測 | 共通・限定条件 | 共通・限定条件 | B6で実受信を共通型へ接続。表示・補間・player rosterは含めない |
| teams／player一覧観測 | 共通・限定条件 | 共通・限定条件 | B6で実宣言・所属移動・各fieldとREMOVEを接続。描画・認証・entity現在位置とは別 |
| その他特殊window／UI／manager | 未共通化 | 未共通化 | 広い対応はB |
| ownedレシピブック配置 | 共通・限定条件 | 共通・限定条件 | B4で通常Next／Maximumを接続。材料不足ghostと実返却も接続。結果のnative QUICK_MOVEは接続済み。広い製作条件はB4へ残る |

A1の固定fixtureはstone床、空のsingle chest、oak planks 2個、収納用stone 2個、
設置用dirt 3個とする。初期配置後にfixtureから操作結果を上書きせず、
同じ接続で短い移動、stoneの収納、2×2でsticksを1回製作、dirtの設置、切断を行う。
通常クリックを使い、レシピブック送信を前提にしない。両mode・両版で同じconsumerを使う。
製作後は入力と結果が空、stick 4個が在庫にあり、cursorが空であることを確認する。
設置後のdirtはSurvivalで2個、Creativeで3個。移動の予測と実受信位置、
close送信と実受信screen履歴を混同せず、サーバー側でも位置・内容・個数・blockを確認する。

### B. 全体を通した後に広げる

以下は最終目標の残作業であり、初回貫通から外したことで完了・不要にはならない。
追加するケースは「どの利用操作が成立するか／どの誤動作を防ぐか」を明記する。

| 段階 | 後続の対応範囲 |
| --- | --- |
| 3 | 広い移動・採掘・設置条件、道具・姿勢・非cube・effect等の対応、観測継続と復旧の範囲拡大、別姿勢／effectを含むCreative飛行後の立位操作への継続 |
| 4 | レシピブック配置とshift転送の残る条件、製作台入力SWAP／QUICK_MOVE、一般装備・entity・item activation、任意item／text／dialogのconstructor・参照・比較と実server cache hash |
| 5 | より広いcontext／記録／再構成／scene／復旧、履歴取得が書き込み停止で詰まる経路の解消、共通遮断後の各履歴取得と不確実性保持、再設定・chunk欠測・再接続の範囲拡大 |
| 6 | 各UI・特殊window・vehicle・manager機能の残差分。raw操作の版依存は明示的な拡張として管理する |

失われるitem data、異なるitemの誤結合、古い接続／画面への送信、取消後の重複送信など、
Aの代表操作で実害がある不足はその操作の前提として先に修正する。
代表操作が使わないconstructorや特殊itemの網羅比較は、その機能を追加する段階で行う。
既存の精密な実装と検証資産は維持し、完了済みの比較を繰り返して進捗に数えない。

### 区切りと検証の運用

- 一つの区切りは、利用者の操作・対応条件・公開API・実受信の結果・失敗時の扱いを揃える。
- 変更した操作の検証を先に行う。新しい変更や失敗がない状態で全suite・全実サーバー・
  package／hash確認を繰り返さない。節目の統合検証と最終配布検証でまとめて行う。
- 元ゲームとの比較は、その操作のcodec・data比較・geometry・判定に必要な依存へ絞る。
  実装から作った同じ期待値だけを使って、版の互換性を確認したとは扱わない。
- build・test・実サーバーは一つずつ実行する。既存の試験harnessと検証記録を再利用する。
- 各区切りで、使えるようになった操作、残る制約、次に進める理由を報告する。
  必要な前提修正が広がる場合は、追加作業の前にロードマップ上の位置と影響を示す。

この順序と代表例に沿って再開する。退避したレシピ配置の変更は初回貫通後のB4で復元した。
初回貫通後も不足はこの文書と対応情報へ残し、全体統合とmain合流の判断を別に行う。

現在の共通Creativeはdefault hotbar write、look/選択、server許可済みのflight requestと4block以内のstep、
loaded/reachable targetへのcreative break/use-on-blockを実装する。
直前のowned飛行stepで既知のdry floorへ戻った後は`land()`から新しい有限地上移動・収納へ継続できる。
着地は正常なstanding・通常attribute・effect／未解決impulseなしに限定し、明示的なlocal stop modelを保持する。
一般飛行の衝突解決や設置成功の保証ではない。
Survivalの追加検査契約は`client.java_1_21_11()?.checked_survival()`で明示的に選ぶ。
moduleは`versions::java_1_21_11::checked`（旧`checked_survival`・`client::survival::checked`は削除）。
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
受信済みの通常item／道具（耐久値だけのdata）と既知dry cube／slab／stairsを扱い、
before-I/O intent、native protocolの違い、fresh target receipt、
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
カーソルに通常itemを持ったQUICK_MOVEも両版へ接続した。cursorのみのdataも保持したregistryで
意味比較し、modernのdata付きcursorは送信revisionを分けてfull実受信を要求する。
特殊item override・製作/一般装備や
後続の統合範囲は継続する。

## レシピ受信の共通化

`Client::received_recipes()`で、全宣言／解放済みdisplayの版差を保持した共通catalogueを取得する。
材料候補・配置・表示結果・解放状態・registry/tag ownerを同じ受信境界で保持し、
未受信のpermissionや実在庫は補完しない。[共通レシピ受信](common-recipes.md)を参照。
recipe選択・計画・配置、記録・再構成・復旧等の残作業を含め、全体goalは継続中。

受信レシピに加えて`Client::recipe_book_materials`でmain/hotbarだけの材料割当を同じcapture境界から取得する。
`ReceivedItem::recipe_book_stock`はnativeの損傷・enchantments・custom name除外と版別stack計数を保持する。
72の元stockケースと80の元ingredient pickerケースで照合する。これは現grid、返却space、UI容量を含む
配置planではなく、stage 4全体の完了も意味しない。詳細は[共通レシピ](common-recipes.md)。

`Client::received_crafting_context()`はplayer・在庫・盤面・recipe/tagを同じ境界で取得する。
`recipe_layout`は両版のnative幾何規則を受信UIへ適用し、`grid_return_plan`は実item dataと
空き容量から盤面だけの仮の返却を計算する。値は`Predicted`であり、cursor返却や送信permissionは
別に扱う。現gridを含む材料割当・UI容量・recipe-book送信・result merge/shift-craftingの
実装は継続する。stage 4と全体goalは未完了。詳細は[共通レシピ](common-recipes.md)。

配置前の返却判定は通常上限超過の実受信countも保持し、旧版の1個ずつの返却と新版の
固定destinationへのsplit/insertを区別する。native splitの入り切らないcopyを
`unreturned_splits()`に保持して`fits()`をfalseにする。損傷道具・変更されたcapacityも
元Inventoryの成功経路へ照合する。送信完了したhotbar選択は`Submitted`のまま計画の
条件として保持し、pending送信や予測を選択の実受信へ昇格しない。

Coherent recipe placement planning now uses `RecipePlacementAmount::{Next, Maximum}`
through the same received crafting context in both versions/modes. It combines
original grid matching, simple main stock and unfiltered input stock, the matched
capacity guard, full safe returns and compatible source data after returns.
Offhand returns cannot be counted as ingredient sources. The public plan remains
read-only, keeps its actual source context and makes no native tie/ACK/consumption
claim. Owned recipe-book submission now retains actual conserved placement or
material-shortage ghosts. Remaining full integration scope is still required.


B4の材料不足ghostは共通観測型とowned要求へ接続した。ghost表示と実盤面・在庫は別に保持し、
実返却と保存が揃うまで操作を解放しない。新版応答にはrecipe IDがないため、表示内容から
選択recipeや因果関係を補完しない。同じ画面番号の再利用・world変更とold planの再送を拒否する。
非空cursorへの結果全体の結合も同じtake APIへ接続した。次のB4区切りはshift製作。一般装備・entity／任意data等を含む
B4全体とA6の利用側検証は継続中。[共通ghost契約](common-recipes.md#received-ghosts-and-conserved-material-shortage-completion)。

Owned ghostは、元playerの`SubmittedClose`から実player zeroの`Received`へ進むことを許可する。
通常結果取得と同じ画面継続規則を使い、planとghostそれぞれの歴史的根拠は書き換えない。
別tableや同番号で開き直したtableの再束縛は拒否する。両版の軽量回帰で確認する。

B4の非空cursor結果結合は、同じitem/data・結合先の実効容量内の結果全体だけを扱う。
実cursor60→結果4→実cursor64→次結果の送信前拒否→格納→空cursorで残り取得を、
両版・両mode・player/tableで同じconsumerから通す。freshな結合cursorとfull盤面を完了条件とし、
消費や次resultを予測で書き換えない。shift製作、table SWAP・QUICK_MOVE、一般装備・entity・data、
広いB3/B5/B6とA6の非公開利用側結果は継続する。


B6の有限digital乗車入力を共通`start_vehicle_control`へ接続した。
地上run完了候補→実乗車→有限入力＋neutral→実下車＋neutral→切断を次の利用単位とする。
runの待機取消・遮断・同数値IDへの再乗車は送信所有と最初の失敗履歴を保持し、
入力送信を車両位置や停止の実受信へ昇格しない。車両の現在状態／paddle／physicsと
下車後の地上継続、その他UI／window／manager、広いB3〜B5とA6利用側結果は残る。
[共通乗車入力](common-vehicles.md#有限の乗車入力)を参照。

両版・両modeの4件の実サーバーworkflowが成功した。元UUIDと乗員受信、実車両移動、
正確な16入力、下車後の重複拒否と切断後の履歴を確認した。
[440入力と検証結果](evidence/common-vehicle-control-20261006.json)を保存した。
この区切りは有限乗車入力であり、広いB6とB3〜B5、A6の利用側結果は継続する。


### B6の受信vehicle motion: 代表フロー完了

有限乗車入力の次に、`Client::entity_motion(EntityId)`で同じ元spawnの位置・回転・
速度sample・ground flagを観測する。spawn履歴と最新fieldの受信根拠を分離する。
今回の完了条件は、同じClientで乗車→有限入力→移動の受信観測→下車を両版・両modeで
通すこと。native physicsの完全再現やconstructorの網羅はこの区切りに追加しない。
下車後の地上継続、特殊minecart補間・paddle、その他UI/window/managerと広いB3〜B5、
非公開利用側のA6結果は残る。[API契約](common-entity-motion.md)。

両版・Survival/Creativeの4ケースが成功した。`trial-1.16.1-3cb7e10d`／
`trial-1.21.11-e3b458d8`でfreshな受信位置の変化と不変のspawn履歴を確認し、
元RCONの車両移動・乗車UUID・下車と独立に照合した。両JVM exit 0、proxy errorsなし。
[固定runtime入力と結果](evidence/common-entity-motion-20261006.json)。


### B6の下車後の地上継続

現在の区切りは、同じClientで接近→乗車→有限入力→実下車とneutral→
`resume_ground`→新しい地上移動→収納→切断を通すこと。
通常の乾いた立位へ戻る利用操作を両mode・両adapterへ接続する。
実受信のown poseから開始し、宣言したzero-controller seedと2tickの予測を
実受信velocityや停止ACKへ読み替えない。
取消・遮断・再乗車・途中の車両消滅の最初の失敗を保持する。
完了済みの地上停止は、元の車両が消滅しても後続地上runへ使用できる。
[API契約](common-dismount-grounding.md)。

次は残る特殊vehicle／window／UI／managerを利用操作の単位で整理し、
広いB3〜B5と非公開A6の固定commit検証を継続する。
車両physicsやconstructorの網羅をこの一区切りの完了条件へ追加しない。

この区切りは両版・両modeの実サーバー4ケースが成功した。
`trial-1.16.1-86c303a4`／`trial-1.21.11-85bf84ae`で、新しい15tick歩行のnative endpointと
実chest OPEN／close／hotbar選択を照合した。両JVM exit 0、proxy errorsなし。
[448入力と検証結果](evidence/common-dismount-grounding-20261007.json)。
この通常地上継続は完了したが、上記の広いBと非公開A6を完了扱いにはしない。

### B6のboss bar観測: 代表フロー完了

共通`Client::boss_bars()`へ、元ADD・progress／name／style／properties更新・REMOVEを
両adapterで接続した。部分更新で無関係なfieldの受信ordinalを更新せず、未知UUIDから
barを作らない。元text encodingとfinite progress、color／overlay、raw flagsを保持する。
[API契約](common-boss-bars.md)。

両版・Survival／Creativeの実ClientでADD→部分更新→REMOVE→manager終了を通した。
`trial-1.16.1-7e474ce2`／`trial-1.21.11-f3351a14`の元packetと独立RCONを照合し、
両JVM exit 0、proxy errorsなし。[451入力と結果](evidence/common-boss-bars-20261007.json)。
次は残るteams／titles／tab list／world border等のUIを、利用操作の単位で進める。
特殊window／vehicle／managerと広いB3〜B5、非公開A6も引き続き必要。
