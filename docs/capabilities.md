# 対応機能と制約

## バージョン別の入口

- Java 1.16.1 / protocol 736: 従来の `Bot` API。以下の従来機能一覧はこの版のものです。
- Java 1.21.11 / protocol 774: `Client` の明示的な版アダプタ。受信ブロック・プレイヤー・所持品、限定的なピストン再構成、通常操作、独立観測付きの歩行とジャンプ、限定的な設置・採掘と接続回復に対応します。
- `Client::survival()` は [検査付きサバイバルAPI](survival-api.md) を選択します。現時点では1.21.11のみがこの契約を実装し、1.16.1には `Unsupported` を返します。従来APIの機能が同じ観測・検査契約を満たすとは扱いません。
- 静的な対応状況は `survival_capabilities()` で取得できます。各操作の現在の可否は、その時点の受信状態と未解決操作から別途判定します。

## Java 1.16.1 の従来API — 対応環境

| 項目 | 対応 |
| --- | --- |
| Minecraft | Java Edition 1.16.1 |
| Protocol | 736固定 |
| 認証 | offline-mode username |
| Runtime | Tokio |
| Bot数 | 1プロセス複数Bot |
| 操作interface | 直接呼び出す型付きRust API |

## 版別の対応

この文書の`Bot`、physics、inventory、survivalの一覧は1.16.1専用です。
1.21.11ではnative block観測、piston再構成、記録、照準、remote player観測と
限定的なクリエイティブ操作、サバイバルでのメイン所持品・ホットバー間の単純スタック交換、
自身の受信状態と静止した通常立位の接地判定を提供します。
通常のサバイバル採掘・配置と、乾いたfull cubeに限定した歩行・ジャンプ制御も追加されています。
移動後の接地は予測と独立接続の観測を照合する契約であり、serverの停止ackではありません。
壁際停止は送信前のclearance確認で拒否し、利用側が退避を入力列へ明示します。
元の失敗と退避後の配置成功の試行記録を保持しています。一般地形の移動や完全な建築executorは提供しません。
詳細は[1.21.11操作](java-1.21.11-operations.md)、[在庫交換](survival-inventory.md)、
[静止・接地判定](survival-standing-context.md)、[採掘](survival-mining.md)、
[配置](survival-placement.md)、[移動制御と失敗記録](survival-motion-controls.md)と
[版別検証](version-adapter-validation.md)を参照してください。

## 実装済み

- 接続、packet圧縮、Keep Alive、Teleport Confirm、respawn、切断
- 前後左右移動、視点、jump、sprint、sneak
- 20 Hz物理、AABB collision、step、液体、登攀、主要特殊block
- chunk palette、block state、biome、light、heightmap/block entity NBT、map item
- block/entity raycast、crosshair target、視線・reach、採掘/設置事前判定
- block stateごとのexact collision box raw registry query（経路選択は含まない）
- bounded・generation-bound raw movement-facts snapshot（player/motion/survival、block shape・registry facts・tool speed map、寸法付きentity、tool/NBT inventory；経路選択は含まない）
- chunk単位の低コピーsnapshot、同一manager内のsection共有・copy-on-write
- health、food、experience、time、weather、effect、attribute、dimension
- player inventory、hotbar、armor、offhand、ItemStack、NBT
- digging、block placement、item use/release、arm swing
- window、cursor、click mode、transaction、reject rollback
- 2×2/3×3 crafting、raw recipe、crafting table
- chest、furnace slotとproperty
- merchant offer、enchant、anvil、beacon、sign、book、horse inventory
- entity spawn/move/teleport/velocity/metadata/equipment/destroy
- entity interaction、attack、cooldown、hurt/death観測
- raw JSON chat、command、player list
- 構造化sound event
- scoreboard、team、boss bar、title、tab、world border、particle、world event
- command tree、server recipes/tags、recipe book、advancement、statistics
- resource pack要求/status、tab completion、NBT query
- passenger、vehicle input/pose、spectator camera、attach/leash
- Bot単位およびmanager単位の物理計測

## Soundについて

Minecraft protocolから取得するのはsound eventです。sound ID/name、category、座標またはentity ID、volume、pitch、sequence、受信時刻を取得できます。

PCM、OGG decode、再生、プレイヤーのマイク入力、voice chat MODの音声は取得しません。

## 意図的に含めない高レベル機能

- A*などの経路探索
- semantic mapと長期world memory
- 自動採掘、resource gathering、建築
- recursive crafting plan
- target選択と戦闘戦略
- LLM、RL policy、behavior treeのruntime
- Bot間の役割決定

これらは公開されたsnapshot、event、operationを利用して上位層に実装します。

## 未対応

- Microsoft/Mojang認証とonline-mode暗号化
- 上記の2版以外のprotocol（版間の暗黙フォールバックなし）
- Forge/Fabric固有handshake
- GUI/render、画面入力、audio playback
- Elytraと完全なvehicle物理
- 厳格なanti-cheatとの完全一致
- plugin固有GUIやmod固有packet
- brewingの専用高レベルhelper（generic window click/propertyは利用可能）
- vehicleの自動物理・操舵方針。入力とpose packetは公開済み

## 実サーバーで確認済みの複合操作

- 木の採掘とblock設置
- 原木から板材、作業台、棒、木のツルハシをcraft
- furnaceで鉄鉱石を精錬して回収
- 空腹状態で食料を使用
- shieldをoffhandへ装備
- cowを追跡して攻撃し、destroyを観測
- 2 Bots間のchatとchest経由のitem受け渡し
- 1、10、50 Bots構成の物理耐久試験
