# 接触dispatchの対応範囲

徒歩・held flight・ボートへの[生物接触の統合](common-entity-owners.md)は、版別 private capability table が資格化した通常scaleの inherited-living / RemotePlayer を対象にする。#92、2026-10-14追加監査と生成した method-owner evidence に基づく有限の範囲で、全speciesの移動対応を意味しない。両版の未変更公式client JARの継承先・原版callbackを確認し、公開するのは以下の解釈と資格化した数値結果である。

| 種類・入口 | Java 1.16.1 / 1.21.11の扱い | 共通ownerの境界 |
| --- | --- | --- |
| Cow / Pig / Zombie / Sheep / Creeper | 原版のliving AI-step、pushEntities / doPush / pushと元worldのlocal-player候補選択 | 完全な履歴・述語・地形が揃う通常scaleを実装。AI/travelや完全なbaseTickは対象外 |
| RemotePlayer | constructorのnoPhysics。modernの速度受信は3段階のlerpMotion | 実profile/game modeが必要。通常pushからlocalへ速度を足さない |
| LocalPlayer | 原版のliving pushはfirst receiverのsleeping、同じvehicle、noPhysics等を検査 | 数値ownerの現在のfeet/速度と、実際のown metadata・profile・mountを使う |
| 自分のBoat / AbstractBoat | 普通のliving pushとsolid collisionは別の入口 | 原版tick slotのliving model box、元のspatial順、spectatorと実passenger除外。車両の固有pushは流用しない |
| 他の固定ボート・トロッコ | 既存PR #74の固定形状と実受信target | ボートの既存laneを維持。remote vehicle travel、特殊トロッコ、自分の徒歩への車両pushは資格化していない |
| ArmorStand | isPushable=false、doPushはno-op。pushEntitiesは近い乗車可能トロッコへ固有dispatch | walking / boat selector から除外。固有minecart laneは別扱い。[#97](https://github.com/sugipamo/Voxrig/issues/97)でmarker/smallと固有車両処理を扱う |
| Bat | isPushable=false、doPush / pushEntitiesはno-op | walking / boat selector から除外。通常Cowへの読み替えをせず、[#97](https://github.com/sugipamo/Voxrig/issues/97)で明示分類する |
| Slime / MagmaCube | Slime.pushは継承pushの後にIronGolem向け固有damage dispatch。MagmaCubeはこの入口を継承 | 拒否。size/overrideは[#98](https://github.com/sugipamo/Voxrig/issues/98)、元のgeometry観測は #40 |
| Shulker | pushはno-op。modernのcanBeCollidedWithはaliveに従う。開閉による動的処理は通常pairと別 | 拒否。接触hookは[#99](https://github.com/sugipamo/Voxrig/issues/99)、元のgeometry観測は #40 |
| その他のspecies / 未知serializer | この監査・owner oracleの対象外 | 保守的なmodel envelopeがqueryと交差すれば拒否。geometry / motionをboundedにできなければ常に拒否 |

## 述語の時点と不足

`alive`、optional sleeping position、pose/babyはspawn constructorの数値初期値と実metadataを元順序で適用する。睡眠はpose enumだけから決めない。睡眠の早期returnは `LivingEntity.push` を受ける側の判定であり、問い合わせ元が眠っているから全batchを除外する規則ではない。原版packetのbedへのfeet変更と、それに続く補間も別々に保持する。

`climbable`はそのAI-step後のモデルfeetと実際に読み込まれた梯子・つた・trapdoorの状態で検査する。playerのspectatorは実受信profile/game mode、teamはplayer名またはspawn UUIDのscoreboard holderと、同じ受信境界のteam状態を使う。missing profile、未検証のstatus/animation/equipment、scale不明・非unit、未知pose/spin hook、地形不足を現在状態の推測で補わない。

vehicle除外は実受信の乗員関係に結び付ける。own boatの乗員をsolid候補から除外する。関係の変更・nested passenger travelは、このモデルの通常living処理へ混ぜない。own correction、world/owner変更、履歴gapと未検証入力はモデルを破棄し、復元後も新ownerを明示して開始する。

## 多数接触とサーバー所有処理

原版の候補順・更新順・浮動小数点の積算順と、SDKの保持上限は別の条件である。owner oracleは8 / 25 / 50 / 100体の混在・自分の前後のslotを含む両版3360段階を比較する。productionの固定上限（tick listの256 active / 512 retained、ボートの32衝突候補・120入力）は、超過を切り捨てず拒否する。

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
owner oracle を両版280 schedule / 3360 stageへ拡張し、実際に重なる個体、
own movement の前後の native slot、baby→adult metadata、push 後の local 速度、
既存の補間・team・sleep・boat geometry を検査する。

再実行は `scripts/run_entity_owner_oracle.py` の既存引数を使い、その work と
ダウンロードを `scripts/audit_sheep_creeper_contact.py` に渡す。
継承先と原版入力hashは
[追加監査記録](evidence/sheep-creeper-contact-20261014.json)、
数値と harness hash は `data/client_api/entity_owner_oracle.json.gz` に記録する。
これは原版 client callback の headless oracle であり、新speciesの live server
接続や完全な mob tick / AI / travel の検証ではない。未監査speciesは private relevance fence に従い、無証明の省略はしない。

## 版別 private capability 生成と deferred relevance

`audit_contact_capabilities.py --downloads <verified-clients> --work <owner-oracle-work>` は
原版 EntityType の class、metadata ancestry と method owner を突き合わせて
owner 内部の table を生成する。ordinary default は監査した LivingEntity / Mob /
Animal / Monster / AgeableMob（legacy AgableMob）/ Sheep の wrapper と、継承push・
寸法・述語・metadata callback が一致したものだけに適用する。
既存の Cow / Zombie / RemotePlayer の資格化済み差異は例外として保持する。
Bat / ArmorStand の no-op・unpushable と vehicle selector、Horse の寸法上限、
Vex の noPhysics wrapper、Item / 監査済みprojectile は別分類する。

[生成 evidence](evidence/contact-capabilities-20261014.json) は両版全登録classの
owner記録とgenerator / output / accessor hashを持つ。数値 oracle は追加された全
inherited classとHorse、既存 Sheep / Creeper の overlap・own前後slot・baby metadataを
比較し、Bat / ArmorStand の原版 virtual doPush / pushEntities と Item / Arrow の
pushability、およびmodern solid exclusionも別記録で検査する。
ArmorStand の特殊minecart挙動や未知dynamic geometryを inherited Cow として
計算する試験ではない。新classのlive-server成功はこのheadless oracleから主張しない。

未知bodyは受信targetをcurrent boxとしてindexへ入れず、owner private stateに保持する。
継承geometry / Horse の保守的hull、入力更新時の失効、tickごとのquery・sweep、寿命/world checkpointと
送信前の停止条件は [owner contract](common-entity-owners.md) の private fence に従う。
公開 selector / geometry API の拒否規則とschemaは変更しない。
