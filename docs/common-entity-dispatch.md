# 接触dispatchの対応範囲

徒歩・held flight・ボートへの[生物接触の統合](common-entity-owners.md)は、通常scaleのplayer / cow / pig / zombieを対象にする。この一覧は #92 の有限の監査結果で、全speciesの移動対応を意味しない。両版の未変更公式client JARの継承先・原版callbackを確認し、公開するのは以下の解釈と資格化した数値結果である。

| 種類・入口 | Java 1.16.1 / 1.21.11の扱い | 共通ownerの境界 |
| --- | --- | --- |
| Cow / Pig / Zombie | 原版のliving AI-step、pushEntities / doPush / pushと元worldのlocal-player候補選択 | 完全な履歴・述語・地形が揃う通常scaleを実装。AI/travelや完全なbaseTickは対象外 |
| RemotePlayer | constructorのnoPhysics。modernの速度受信は3段階のlerpMotion | 実profile/game modeが必要。通常pushからlocalへ速度を足さない |
| LocalPlayer | 原版のliving pushはfirst receiverのsleeping、同じvehicle、noPhysics等を検査 | 数値ownerの現在のfeet/速度と、実際のown metadata・profile・mountを使う |
| 自分のBoat / AbstractBoat | 普通のliving pushとsolid collisionは別の入口 | 原版tick slotのliving model box、元のspatial順、spectatorと実passenger除外。車両の固有pushは流用しない |
| 他の固定ボート・トロッコ | 既存PR #74の固定形状と実受信target | ボートの既存laneを維持。remote vehicle travel、特殊トロッコ、自分の徒歩への車両pushは資格化していない |
| ArmorStand | isPushable=false、doPushはno-op。pushEntitiesは近い乗車可能トロッコへ固有dispatch | 拒否。[#97](https://github.com/sugipamo/Voxrig/issues/97)でmarker/smallと固有車両処理を扱う |
| Bat | isPushable=false、doPush / pushEntitiesはno-op | 拒否。通常Cowへの読み替えをせず、[#97](https://github.com/sugipamo/Voxrig/issues/97)で明示分類する |
| Slime / MagmaCube | Slime.pushは継承pushの後にIronGolem向け固有damage dispatch。MagmaCubeはこの入口を継承 | 拒否。size/overrideは[#98](https://github.com/sugipamo/Voxrig/issues/98)、元のgeometry観測は #40 |
| Shulker | pushはno-op。modernのcanBeCollidedWithはaliveに従う。開閉による動的処理は通常pairと別 | 拒否。接触hookは[#99](https://github.com/sugipamo/Voxrig/issues/99)、元のgeometry観測は #40 |
| その他のspecies / 未知serializer | この監査・owner oracleの対象外 | 遠さを理由に安全な通常生物へ補わず拒否 |

## 述語の時点と不足

`alive`、optional sleeping position、pose/babyはspawn constructorの数値初期値と実metadataを元順序で適用する。睡眠はpose enumだけから決めない。睡眠の早期returnは `LivingEntity.push` を受ける側の判定であり、問い合わせ元が眠っているから全batchを除外する規則ではない。原版packetのbedへのfeet変更と、それに続く補間も別々に保持する。

`climbable`はそのAI-step後のモデルfeetと実際に読み込まれた梯子・つた・trapdoorの状態で検査する。playerのspectatorは実受信profile/game mode、teamはplayer名またはspawn UUIDのscoreboard holderと、同じ受信境界のteam状態を使う。missing profile、未検証のstatus/animation/equipment、scale不明・非unit、未知pose/spin hook、地形不足を現在状態の推測で補わない。

vehicle除外は実受信の乗員関係に結び付ける。own boatの乗員をsolid候補から除外する。関係の変更・nested passenger travelは、このモデルの通常living処理へ混ぜない。own correction、world/owner変更、履歴gapと未検証入力はモデルを破棄し、復元後も新ownerを明示して開始する。

## 多数接触とサーバー所有処理

原版の候補順・更新順・浮動小数点の積算順と、SDKの保持上限は別の条件である。owner oracleは8 / 25 / 50 / 100体の混在・自分の前後のslotを含む両版768段階を比較する。productionの固定上限（tick listの256 active / 512 retained、ボートの32衝突候補・120入力）は、超過を切り捨てず拒否する。

crammingのサーバー側damage/RNG、mob AI、攻撃、server travelをクライアント予測から確定・送信しない。通常の操作統合に必要なRemotePlayerのnoPhysics、sleep callback、passenger除外とboat固有selectorは実装済みだが、species全般の対応は個別issueが残る。追加姿勢は #93、追加の乗り物は #94、観測/codecは #40へ分ける。

原版入力のhash、owner oracle・公式サーバー接続と失敗試行は[検証記録](evidence/common-entity-owners-20261012.json)に対応付ける。特殊speciesの上記4行は継承/bytecode監査であり、そのspeciesの原版軌跡・実接続成功を報告するものではない。原版JAR・mapping・bytecode・method bodyは公開しない。
