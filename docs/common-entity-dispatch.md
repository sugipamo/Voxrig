# 接触dispatchの対応範囲

現在の資格集合は private const table から生成した次の表で確認する。
`python3 -B scripts/contact_policy_export.py --check` が集合と生成内容を検査する。
公開 geometry・packet・locality の別契約は[consumer manifest](contact-policy-consumers.md)、
原版数値 corpus の全 scenario ID は `check_contact_policy_consumers.py --check` で照合する。
Wolf は Unknown のまま、Step 6 (#128 option 3) では contact の影響を省略して制御を継続する。
[動作変更と API 移行](contact-policy-consumers.md#step-6-intentional-pass-through-issue-128-option-3)を参照。

<!-- contact-policy:selectors:start -->
| Version | Owned tick selector | Qualified species |
| --- | --- | --- |
| 1.16.1 | Chicken | chicken |
| 1.16.1 | InheritedLiving | cow, creeper, giant, mooshroom, ocelot, pig, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.16.1 | RemotePlayer | player |
| 1.21.11 | Chicken | chicken |
| 1.21.11 | InheritedLiving | cow, creeper, giant, mooshroom, ocelot, pig, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.21.11 | RemotePlayer | player |
<!-- contact-policy:selectors:end -->

## Historical audit notes (before Chicken qualification)

以下は Step C/D 当時の監査記録である。種別列挙・件数・Chicken 未資格の記述は
当時の状態を示し、現在の資格一覧ではない。現在の資格は上の生成表と
[capability audit](contact-capability-audit.md)、数値 corpus は固定 gz fixture と
[consumer checks](contact-policy-consumers.md)を参照する。副作用・限界の監査判断は保持する。


徒歩・held flight・ボートへの[生物接触の統合](common-entity-owners.md)は、版別 private capability table が資格化した通常scaleの inherited-living / RemotePlayer を対象にする。#92、2026-10-14/15追加監査と生成した method-owner evidence に基づく有限の範囲で、全speciesの移動対応を意味しない。両版の未変更公式client JARの継承先・原版callbackを確認し、公開するのは以下の解釈と資格化した数値結果である。

| 種類・入口 | Java 1.16.1 / 1.21.11の扱い | 共通ownerの境界 |
| --- | --- | --- |
| Cow / Pig / Zombie / Sheep / Creeper / Rabbit / Ocelot / Villager / WanderingTrader / Mooshroom / Silverfish / Giant | 原版のliving AI-step、pushEntities / doPush / pushと元worldのlocal-player候補選択 | 完全な履歴・述語・地形が揃う通常scaleを実装。AI/travelや完全なbaseTickは対象外 |
| RemotePlayer | constructorのnoPhysics。modernの速度受信は3段階のlerpMotion | 実profile/game modeが必要。通常pushからlocalへ速度を足さない |
| LocalPlayer | 原版のliving pushはfirst receiverのsleeping、同じvehicle、noPhysics等を検査 | 数値ownerの現在のfeet/速度と、実際のown metadata・profile・mountを使う |
| 自分のBoat / AbstractBoat | 普通のliving pushとsolid collisionは別の入口 | 原版tick slotのliving model box、元のspatial順、spectatorと実passenger除外。車両の固有pushは流用しない |
| 他の固定ボート・トロッコ | 既存PR #74の固定形状と実受信target | ボートの既存laneを維持。remote vehicle travel、特殊トロッコ、自分の徒歩への車両pushは資格化していない |
| ArmorStand | isPushable=false、doPushはno-op。pushEntitiesは近い乗車可能トロッコへ固有dispatch | walking / boat selector から除外。固有minecart laneは別扱い。[#97](https://github.com/sugipamo/Voxrig/issues/97)でmarker/smallと固有車両処理を扱う |
| Bat | isPushable=false、doPush / pushEntitiesはno-op | walking / boat selector から除外。通常Cowへの読み替えをせず、private table でこの lane の除外を証明する |
| Slime / MagmaCube | Slime.pushは継承pushの後にIronGolem向け固有damage dispatch。MagmaCubeはこの入口を継承 | 拒否。size/overrideは[#98](https://github.com/sugipamo/Voxrig/issues/98)、元のgeometry観測は #40 |
| Shulker | pushはno-op。modernのcanBeCollidedWithはaliveに従う。開閉による動的処理は通常pairと別 | 拒否。接触hookは[#99](https://github.com/sugipamo/Voxrig/issues/99)、元のgeometry観測は #40 |
| その他のspecies / 未知serializer | Step D の未資格化理由は追加監査表を参照。未監査hookは補完しない | 未資格の生物は table の PassThrough。modeled body の未知 serializer 等は従来どおり拒否 |

## 述語の時点と不足

`alive`、optional sleeping position、pose/babyはspawn constructorの数値初期値と実metadataを元順序で適用する。睡眠はpose enumだけから決めない。睡眠の早期returnは `LivingEntity.push` を受ける側の判定であり、問い合わせ元が眠っているから全batchを除外する規則ではない。原版packetのbedへのfeet変更と、それに続く補間も別々に保持する。

`climbable`はそのAI-step後のモデルfeetと実際に読み込まれた梯子・つた・trapdoorの状態で検査する。playerのspectatorは実受信profile/game mode、teamはplayer名またはspawn UUIDのscoreboard holderと、同じ受信境界のteam状態を使う。missing profile、未検証のstatus/animation/equipment、scale不明・非unit、未知pose/spin hook、地形不足を現在状態の推測で補わない。

vehicle除外は実受信の乗員関係に結び付ける。own boatの乗員をsolid候補から除外する。関係の変更・nested passenger travelは、このモデルの通常living処理へ混ぜない。own correction、world/owner変更、履歴gapと未検証入力はモデルを破棄し、復元後も新ownerを明示して開始する。

## 多数接触とサーバー所有処理

原版の候補順・更新順・浮動小数点の積算順と、SDKの保持上限は別の条件である。原版 fixture は両版320 schedule / 3840 stageを保持する。SDKとの数値比較は資格化した208 schedule / 2496 stage（walking / boat 各1248）で、8 / 25 / 50 / 100体の混在・自分の前後のslotを含む。残る未資格speciesを通常livingとして比較・admitしない。productionの固定上限（tick listの256 active / 512 retained、ボートの32衝突候補・120入力）は、超過を切り捨てず拒否する。

crammingのサーバー側damage/RNG、mob AI、攻撃、server travelをクライアント予測から確定・送信しない。通常の操作統合に必要なRemotePlayerのnoPhysics、sleep callback、passenger除外とboat固有selectorは実装済みだが、species全般の対応は個別issueが残る。追加姿勢は #93、追加の乗り物は #94、観測/codecは #40へ分ける。

原版入力のhash、owner oracle・公式サーバー接続と失敗試行は[検証記録](evidence/common-entity-owners-20261012.json)に対応付ける。特殊speciesの上記4行は継承/bytecode監査であり、そのspeciesの原版軌跡・実接続成功を報告するものではない。原版JAR・mapping・bytecode・method bodyは公開しない。

#93/#94の調査・分割結果は[姿勢](common-posture-coverage.md)と[追加mount](common-mount-coverage.md)の有限一覧に記録した。sleep/Riptide・追加操縦の各実装childは未完了である。

## Sheep / Creeper の追加監査（2026-10-14）

Java 1.16.1 / 1.21.11 の hash 検証済み未変更公式 client を使い、
Sheep / Creeper の `pushEntities` / `doPush` / `push` / `isPushable` と
`getDimensions` が `LivingEntity` に到達することを確認した。
Sheep の `aiStep` は草食アニメーションを更新して継承先へ委譲する。
`AgableMob`（1.16.1）/ `AgeableMob`（1.21.11）の client 側 age 処理は
表示用 timer/particle で、baby metadata の callback は寸法を更新する。
Creeper の膨張表示・爆発を、この限定した inherited contact lane の travel や
push override として代用しない。

通常scaleの Sheep は幅 `0.9f`・高さ `1.3f`、baby はそれぞれ `0.5f` 倍。
Creeper は幅 `0.6f`・高さ `1.7f`。既存の private admission と baby metadata
reducer を使い、公開 geometry API の対象範囲は変更しない。
原版 fixture は両版320 schedule / 3840 stageを保持する。資格化した208 schedule /
2496 stageのSDK比較で、実際に重なる個体、own movement の前後の native slot、
baby→adult metadata、push 後の local 速度、既存の補間・team・sleep・boat geometry を検査する。

再実行は `scripts/run_entity_owner_oracle.py` の既存引数を使い、その work と
ダウンロードを `scripts/audit_sheep_creeper_contact.py` に渡す。
継承先と原版入力hashは
[追加監査記録](evidence/sheep-creeper-contact-20261014.json)、
数値と harness hash は `data/client_api/entity_owner_oracle.json.gz` に記録する。
これは原版 client callback の headless oracle であり、新speciesの live server
接続や完全な mob tick / AI / travel の検証ではない。未監査speciesは typed Unsupported として拒否し、無証明の省略はしない。

## 版別 private capabilities (Step C)

`audit_contact_capabilities.py --downloads <verified-clients> --work <audit-work>` は
hash 検証した原版 client / Mojang mappings から remap し、版別 method owner と
監査対象 class・全 ancestor の method fingerprint を生成する。
[副作用監査](contact-capability-audit.md)と
[生成 evidence](evidence/contact-capabilities-20261014.json)を対応付ける。
Player は EntityType の generic Player ではなく実際の RemotePlayer class を確認する。

walking outgoing push、incoming pushability、solid collision、geometry と
metadata / status / animation の各 callback を独立に分類する。
既存の六種類だけを Qualified owner lane に入れ、Bat / ArmorStand は
contact exclusion を ProvenIrrelevant とする。その他は Unknown または
部分監査の BoundedUnknown のまま PassThrough とする。継承先が一致するだけでは資格化しない。
Status / animation は Step A の有限な no-op 証明以外、効果を適用せず無視する。

Step B の[有限 envelope 証明](contact-envelope-audit.md)は capability table と独立で、
まず default / unmounted Spider・CaveSpider のみへ適用する。
受信 target を current geometry とせず、spawn 以降の全補間 anchor・codec base・
absolute endpoint を含む縮まない hull を使う。own 前後 slot・swept / step-up query・
boat selector に加え、全 qualified remote の補間 / shifted target と固定 boat collider
からの非交差を要求し、間接作用も除外する。これは証明済み無関係性の条件である。
Unknown hook は近傍限定と仮定しない。metadata / scale / equipment / status /
animation / mount で有限証明を失えば、PassThrough へ戻る。
その他の Unknown / BoundedUnknown も数値 body を作らず contact の影響を省略する。

元の native slot と spawn lifetime を保持し、除去・再利用・metadata・乗員・checkpoint /
world 境界で再評価する。checkpoint は累積 hull と失効原因 packet を保存する。
真の admission / 数値処理失敗時の staged frame 送信禁止と入力解除、公開型 / schema は変更しない。
原版 numerical fixture は既存六種類の own前後slot・baby metadata、補間・team・sleep・
boat geometry と Bat / ArmorStand exclusion を引き続き検査する。
追加 species の envelope 証明は数値 body や全 callback の資格化ではない。


## Step D: ordinary mob qualification (2026-10-15)

The private owned contact lane additionally qualifies Rabbit, Ocelot, Villager,
WanderingTrader, Mooshroom, Silverfish and Giant in **both 1.16.1 and 1.21.11**.
Cow/Pig/Zombie/Sheep/Creeper/RemotePlayer retain their numerical regressions.
The public geometry API and owner origin, history, unit-scale, packet and mount
guards are unchanged. This does not qualify complete NPC ticks or server AI.

[The capability audit](contact-capability-audit.md#step-d-ordinary-mob-review)
records each accepted type's original owners/side effects and every reviewed
rejected type's blocker, with [dated evidence](evidence/contact-capabilities-20261015.json)
and reproducible original-client oracle commands. In particular Chicken changes
falling velocity and has modern custom baby geometry; Cat/Wolf resolve owner
teams differently by version; Fox sleeping flags and horse-family geometry
need additional modeled state. They remain unqualified. All other unclosed
reviews likewise retain Unknown (Horse's prior partial outgoing annotation
still admits nothing). No ancestry or reason-string dispatch is used.

The fixture has 320 schedules / 3,840 stages. Exact SDK comparison covers 208
qualified schedules / 2,496 stages, split equally between walking and boat;
the 1,344 unqualified raw stages are excluded from numerical claims. Baby/adult
and species-variant inputs are tested where applicable. Step 6 now permits
unqualified species near and far by omitting their contact influence. Existing
Spider/CaveSpider Step B proofs remain separate evidence of nonintersection;
proof failure falls back to PassThrough. No unqualified species is promoted.

Legacy dimension-refresh boxes retain their original minimum on equal/growing
width until a real position placement; modern refresh recenters immediately.
This state is private and checkpointed. Exact raw bounds and boat comparisons
cover sleeping/standing transitions, including legacy Silverfish seed 49 stage 7.
Step B's existing peer hull now also encloses the next recentered placement;
no unqualified provider or callback gains an envelope exemption.
