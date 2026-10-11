# 共通ClientのElytra滑空

`client.player_control(mode).start_glide_control(Controls)`は、地上操作・
Creative/Spectatorのheld flightと別の`ControlKind::Glide` ownerを開始する。
Survival/Adventure/Creativeの健康なplayer、現在worldの実受信chest Elytra、
原版の使用可能な耐久値、unit scale、既知の乾いた空中起点が必要。
地上、乗車中、既に実際に滑空中、装備欠測・不明なcomponent、飛行能力の使用中は拒否する。
この操作は経路計画やロケットboostを行わない。

## 開始要求と実受信

開始意図を保存したconnection所有taskが一度だけ原版のSTART_FALL_FLYINGを送る。
legacy PlayerCommand actionは8、modernは6であり、両版に同じ番号を流用しない。
`ControlRecord.glide.dispatched`は完全writeだけを示す。
`accepted_sequence`は要求のfenceより後に受信したown entity flag bit 7の最初のsetで、
開始要求へのACKや因果関係の証明ではない。実flagが届くまで位置を予測・送信しない。
40tickで未確認のownerを終了し、不確かな要求を自動再送しない。

`PlayerObservation.fall_flying`と`GlideRecord.actual`は実受信値と元ordinalを保つ。
`ControlFrame.gliding`、FallFlying姿勢、箱とeyeは数値モデルであり、サーバーの現在位置ではない。
装備のoriginal slot receiptとnative durability predicatesも保存する。
legacyのisFlyEnabledとmodernのnextDamageWillBreakは別の原版条件を使う。

## 操作と終了

同じIDを指定した`set_controls_for`でyaw/pitchを変更する。
原版のlift・降下・水平加速・drag・壁衝突・着地を実装し、FallFlyingの箱は高さ0.6、eyeは0.4。
実際のflag解除、装備喪失、補正、死亡、world/mode/scale変更、乗車、切断はownerを終了する。
flagや装備が次tick前に復元しても、最初の終了ordinalを保持して再開しない。
地形・入力・履歴・再構成が不明になった滑空も終端停止し、古い起点から再試行しない。
IDは元の接続/worldに限定され、保存recordを別の接続へ移して操作を許可しない。

`stop_control_for`は入力writerを解除する。原版にはSTOP_FALL_FLYING packetがなく、
STARTを再送して停止toggleに使わない。owner停止は物理的な滑空終了・静止の保証ではない。
自然着地または明示した降下を実行し、実flag解除とStanding姿勢を確認した後に、
新しい地上ownerを明示的に開始する。古いGlide IDはそのownerを操作できない。
キャンセルした待ち手からtaskの寿命を切り離し、writeの失敗・不確実性は記録に残す。
共通transportの部分write保護と再送禁止を継承する。

## 資格化した範囲

乾いた静的地形、vanilla chest Elytra、unit scaleが対象。
1.21.11の高速移動に伴う落下距離リセットのclipは、原版の最大8block区間の
保守的な包絡に水・climbable・cobweb・sweet berry bushがない場合だけMISSを確定する。
候補や欠測があれば移動前に拒否する。水中のclip hitや任意のserver tag変更は資格化していない。
水・lava、ロケットboost、独自glider/equippable、別装備slot、非unit滑空は未対応。
未対応entity/hookとの接触は一般化せず、通常滑空として続行しない。
単発のアイテム使用・到達距離・採掘などは既存のStanding操作契約を維持する。

## 検証

56場面・1,792tickを、未変更の公式Player/LivingEntityのtick/travel/poseと比較する。
yaw/pitch、姿勢とeye/箱、Slow Falling、壁、着地、速度2.5の降下を含む。
原版server terrain上の数値originで、original Levelのclient-side分岐をtick中だけ選択し、
終了後に復元する自作wrapperを用いる。既存の自作LocalPlayer入力portは維持する。
グラフィカルClientLevelのpacket時計やサーバー受理をこの比較から主張しない。
元harness/runnerは変更せず、生成copy、JAR/mappings、入力・結果のhashを保存する。

同じ固定common Client binaryで公式1.16.1の17項目・1.21.11の20項目、計37項目を通過した。
実装備・元START packet・
実flag/ordinal・予測と送信座標bytes・独立RCON位置を照合する。
着地と後続地上操作、壁、入力解除、装備喪失、補正、死亡/respawn、古いIDの拒否、
modernの強制乗車/下車を確認する。両版TCPテストでは未確認開始のtimeoutと無再送、
modernでは待機取消し・乗車停止・送信前のconnection失効を確認する。
完全な部分write保護は共通transportの既存TCPテストでも検査する。

[source/build/binary/JAR・実接続記録](evidence/common-glide-20261012.json)には
失敗した初回driverと未対応高速降下の実行も残す。
再現は`run_glide_oracle.py`と`run_glide_live.py`を使用する。
