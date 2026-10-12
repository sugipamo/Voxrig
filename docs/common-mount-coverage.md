# 乗車可能性と追加操縦の有限な一覧

#94は原版の乗車・操縦条件を調査して分割するissueであり、追加mountを実装済みにしない。
`VehicleObservation`は実Passengers順序とMountIdを保持する。
乗車関係の観測、通常のride可能性、操縦席、local movement authorityは別の条件である。
commandの強制乗車で任意entityへ乗れても、通常の操縦能力を与えない。

## 種類・版・現在の支持範囲

| 種類 | 1.16.1 / 1.21.11 | 席・装備・原版の操作分岐 | 現行と実装issue |
| --- | --- | --- | --- |
| Boat | 両版 | 原版controlling passenger、二席、独自の水/泡/paddle/local tick | 既存共通owner。追加実装しない |
| Chest Boat / Raft / Chest Raft | modernのみ | AbstractBoatの席/controlling分岐。cargoや型は別に保持 | 既存boat型の支持範囲。legacyへ型を作らない |
| 乗車用Minecart | 両版 | server駆動、実乗車と入力。VehicleMoveのlocal boat物理を流用しない | 既存有限入力/解除/下車。特殊modern受信sampleは #40 |
| 他のminecart variant | 両版、型は版別 | cargo/furnace/TNT等は通常player席とは別。forced mountは乗員観測と操縦可否を分離 | 既存型/receiptの範囲を維持、任意variantの操縦を約束しない |
| Horse / Donkey / Mule / Skeleton Horse / Zombie Horse | 両版 | 原版tame/saddle・controlling first passenger、ridden travel、jump charge | 未対応、[#105](https://github.com/sugipamo/Voxrig/issues/105) |
| Pig | 両版 | saddle、実carrot-on-a-stick、controller、前進一定のridden分岐、boost | 未対応、[#106](https://github.com/sugipamo/Voxrig/issues/106) |
| Strider | 両版 | saddle、実fungus-on-a-stick、controller、lava/cold、前進一定、boost | 未対応、[#107](https://github.com/sugipamo/Voxrig/issues/107) |
| Camel / CamelHusk | modernのみ | AbstractHorseを継承、二席・鞍・前席操縦、sitting/standing・dash/cooldown | 未対応、[#108](https://github.com/sugipamo/Voxrig/issues/108) |
| Happy Ghast | modernのみ | adult、harness/body equipment・原版control eligibility、四席・first controller、専用飛行入力 | 未対応、[#109](https://github.com/sugipamo/Voxrig/issues/109) |
| Nautilus / ZombieNautilus | modernのみ | AbstractNautilusを継承、saddle/first controller、水/空気の速度・dash/jump | 未対応、[#110](https://github.com/sugipamo/Voxrig/issues/110) |
| Llama / TraderLlama | 両版 | 馬系の乗車と操縦可否は別。通常の鞍操縦をHorseと同一視しない | 支持範囲・下車/起点を[#111](https://github.com/sugipamo/Voxrig/issues/111)で確定 |
| Ravager / 非操縦席 / 強制command mount / その他 | 種類・版別 | getterが乗員を返すことだけでは通常のplayer ride/travelを保証しない。server AIとlocal authorityを個別判定 | generic実関係の観測を維持。操縦は拒否、下車/起点の不足は #111 |

追加の操縦classを名前だけで約束しない。原版に不存在、操縦不能、未対応を区別する。
小さなvariantの継承があっても、固有装備・seat・環境の条件を検査してから支持する。

## 原版のauthorityとpacket

原版Entity/LivingEntity/Mobと各mountは、controlling passengerやlocal-instance/
canSimulateMovementの条件でtravelを分岐する。通常のAIはserver所有。
実際に自分がlocal controllerになった分岐では、その種別のridden physicsと
VehicleMoveを比較する必要がある。server所有の動きへBoatの数値物理を流用しない。

legacyの操縦入力はnative PlayerInputのstrafe/forward/jump/sneak、
modernはnative PlayerInputのkey bitsを使う。VehicleMove、Boatのpaddle、
PlayerCommandのjump/下車は別のpacket・所有契約である。
START_RIDING_JUMP / STOP_RIDING_JUMPはlegacy 5/6、modern 3/4。
下車入力も版別で、legacyのshift action番号をmodernへ流用しない。
原版enumの実ordinalは監査記録に保存した。

Pig/Striderのmodern getRiddenInputは前進一定のvectorである。
そのためneutral keyのwrite完了を実停止へ読み替えない。
boostのserver状態、stick保持やcontrollerの条件を確認し、操作解除と独立した実静止の
観測を契約に含める。Horse jump、Camel dash、Nautilus dash、Happy Ghast flyも
requestの送信を受理/移動成功と扱わない。

## 共通の不足と実装条件

各childには実spawn/Passengers順序、connection/world/seat/owner寿命、
操縦装備・属性・状態、neutralと実停止、下車・forced dismount、席交代、
cancel/部分write/補正/死亡/旧ID・無再送の検証条件を記載した。
受信codec・geometryの不足は #40と分担し、未知入力をvanilla/defaultに補わない。

既存ownerは未知mountの操縦を拒否し、実乗車で古い徒歩/飛行/滑空起点を破棄する。
下車関係を受信しただけではStanding/足場/速度の新しい地上起点を保証しない。
強制下車後のtpや待機だけで古いownerを再開しない。
この保護は[#91のmodern実接続](common-glide-control.md)で再確認したが、
Pig操縦や追加mount全体の実接続証明ではない。

## 調査の証拠と境界

[監査記録](evidence/movement-capability-audit-20261012.json)は、公式JAR/mappingのhash、
原版runtime enum、entity namespace内の671 / 1,100 classの宣言、直接の
controlling-passenger宣言（legacy 6 / modern 8 class）、選定hookのmethod hashと
CamelHusk/Nautilus/ZombieNautilusの継承を記録する。class数はentity種類数ではない。
Camel等の継承したcontrollerと、特殊ridden inputを区別した有限分類である。

method/branch監査であり、新しいmountの数値oracle・実接続成功として数えない。
各childの実装時に、未変更の原版物理と該当版公式serverの
実seat/equipment・元packet・独立RCON・終了後の新しい地上起点を追加検証する。
#44は追加実装issueが残る間は開いたままにする。
再現scriptは[audit_movement_capabilities.py](../scripts/audit_movement_capabilities.py)。
原版バイナリ・mapping・bytecode・method bodyは配布しない。
