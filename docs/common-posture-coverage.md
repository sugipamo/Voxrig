# player姿勢の有限な対応一覧

#93は姿勢を調査・分割するissueであり、追加姿勢の移動実装の完了を示さない。
原版Pose enumの実ordinal/serializerの25項目は既存のpose oracleで検証済み。
今回は両版Playerの姿勢選択とsleep/spin処理を調べ、追加実装を
[睡眠・起床 #103](https://github.com/sugipamo/Voxrig/issues/103)と
[Riptide #104](https://github.com/sugipamo/Voxrig/issues/104)へ分けた。

## 原版の全poseと自分の操作

ordinalは版別。metadataの列挙番号からplayerに適用可能とは判断しない。

| 原版pose | 1.16.1 | 1.21.11 | 自分のplayerと現行操作 |
| --- | --- | --- | --- |
| STANDING | 0 | 0 | 地上・通常のCreative/Spectator flightで対応 |
| FALL_FLYING | 1 | 1 | [専用Glide owner](common-glide-control.md)。vanilla chest/unit/dryに限定 |
| SLEEPING | 2 | 2 | Playerのbed/sleep/wake。操作未対応、#103 |
| SWIMMING | 3 | 3 | 水泳・姿勢fitによるcrawlで対応。狭い地形のfallbackであり新poseではない |
| SPIN_ATTACK | 4 | 4 | PlayerのRiptide。操作・impulse/timer未対応、#104 |
| CROUCHING | 5 | 5 | 地上のsneak・姿勢fitで対応 |
| DYING | 6 | 7 | 死亡でownerを終了。生存playerの継続操作モデルにしない |
| LONG_JUMPING | 不存在 | 6 | mob用。Playerのdesired poseに選ばれない |
| CROAKING | 不存在 | 8 | Frog等のmob用。Player操作へ適用しない |
| USING_TONGUE | 不存在 | 9 | Frog等のmob用。Player操作へ適用しない |
| SITTING | 不存在 | 10 | Camel等のmob用。playerの乗車や座標からposeを生成しない |
| ROARING | 不存在 | 11 | Warden等のmob用。Player操作へ適用しない |
| SNIFFING | 不存在 | 12 | Warden/Sniffer等のmob用。Player操作へ適用しない |
| EMERGING | 不存在 | 13 | mob用。Player操作へ適用しない |
| DIGGING | 不存在 | 14 | mob用。Player操作へ適用しない |
| SLIDING | 不存在 | 15 | Breeze等のmob用。Player操作へ適用しない |
| SHOOTING | 不存在 | 16 | Breeze等のmob用。Player操作へ適用しない |
| INHALING | 不存在 | 17 | Breeze等のmob用。Player操作へ適用しない |

生物ごとのbox/eye、baby/scale、metadata・補間、sleep callbackは #40の観測契約。
上表のmob例はplayer操作との分類であり、全speciesの形状・移動・接触を資格化していない。
接触に使える種類とhookは[dispatch一覧](common-entity-dispatch.md)に従う。

## 遷移とowner

legacy Playerはfit検査の後、fall flying → sleeping → swimming → spin →
非flyingのsneak → standingの順にdesired poseを選ぶ。
modernはsleeping → swimming → fall flying → spin → 非flyingのsneak → standing。
Spectatorやpassengerを除くfit不足では、crouchが入るならcrouch、次にswim/crawlへfallbackする。
これらの条件はnativeの状態であり、SDKは実受信状態とモデルの出力を別々に保持する。
この監査は原版の矛盾する同時flagを任意のpriorityでSDKへ許可するものではない。

睡眠はpose enumだけでは成立せず、実optional bed位置と原版sleep/wake callbackが必要。
wake actionはlegacy 2、modern 0。要求writeを実際の起床へ読み替えない。
Riptideには実trident/enchantment、charge/release、水/雨、native impulseとspin flag/timerが必要。
単なるSPIN_ATTACK metadataを通常の地上速度に混ぜない。

現行のown contact ownerはsleep/spin/未知poseを拒否・終端停止する。
Glide以外のground/flightは実fall-flyingを検知して終了する。
未知入力・履歴欠落でモデルを破棄した後は、復元しても古いownerを再開せず、
健康・現在world・対応poseと新しい起点を確認して明示開始する。
死亡やworld変更を受けた旧IDを新ownerへの操作許可にしない。
Boatのpassenger offsetはplayerの追加poseモデルではなく、乗車ownerの別処理である。

## 調査の証拠と境界

[監査記録](evidence/movement-capability-audit-20261012.json)は、未変更の公式JARの
runtime enum、classfileの宣言/継承・method hashを保持する。
既存の[pose受信の原版比較・実接続](evidence/common-entity-poses-20261011.json)と、
[scale #90](common-player-scale.md)・[Glide #91](common-glide-control.md)へ対応付ける。
今回の調査を新しいsleep/Riptideのoracle・実接続成功として数えない。
原版バイナリ・mapping・bytecode・method bodyは配布しない。

再現: `audit_movement_capabilities.py --downloads <official-jars-and-mappings> --work <local> --output <json>`。
公式JAR/mappingのhashを検査し、原版のenumを実行、原版classfileの宣言を読む。
元pose oracleとの25項目一致を確認し、個別childの実装時に軌跡と実接続を追加する。
