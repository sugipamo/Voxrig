# 自分のplayerのscale付き継続操作

Java 1.21.11の共通Clientの地上・held flightは、実際のscale属性の計算値を
0.125〜4の範囲で使う。全範囲を証明したものではなく、原版の0.0625〜16のうち
この範囲を資格化している。1.16.1にはscale属性がなく、unit寸法を使う。

## 受信値と予測

`PlayerObservation.attributes["minecraft:scale"]`は元のbase・modifier・valueと
source/receive ordinalを保つ。原版属性の3種類のmodifier演算とclampを経た値を、
数値モデルへ変換する。`ControlFrame.model_scale`は原版のfloat変換後の値、
`bounds`はmin XYZ / max XYZ、`eye_position`はモデル上の目の位置である。
これらは受信box・サーバー位置・操作の承認ではない。

初期constructorのunit値は既知のnative resetとして区別される。
非unitの開始には現在worldの実属性が必要。欠測、NaN、範囲外、legacyの非unitは
移動packetを送る前に拒否する。実装は寸法・姿勢・目の高さ、modernの微小速度閾値、
block押し出し、honeyの半幅に原版の演算順を適用する。
step heightは元のattribute値であり、移動速度・jump・gravity等へ独自倍率を掛けない。

## ownerの寿命

開始時にscaleをモデルへ取り込み、生物接触の最初のtickより前に箱を設定する。
同じ値の再受信はモデルを切らない。値の変化は最初の受信ordinalを
`scale_change_sequence`へ保存し、そのownerを終端停止する。2→1→2の復元が
次のモデルtickより早くても自動再開しない。実観測を確認し新しいownerを開始する。
古いIDによる操作は新しいownerへ作用しない。world・死亡・補正・乗車・切断などの
既存の停止境界、部分writeと再送禁止も維持する。

他生物の通常scale接触の資格化は[生物接触owner](common-entity-owners.md)に従う。
他生物の非unit scaleや特殊hookは一般化していない。
到達距離・単発の採掘/インタラクション・有限path・乗車のscale対応は別契約で、
既存のunit制限を一括解除しない。

## 検証

自作wrapperは既存movement harnessを変更せず、private copyへ観測と
実属性/modifier適用後の原版`refreshDimensions`呼出しだけを追加する。
未変更の公式Player/LivingEntityのtravel・pose・衝突処理を実行し、
legacy4場面・modern69場面、計73場面・1,752tickを位置・速度・箱・姿勢・eyeのbitsで照合する。
6段階のscale、壁・段差・狭い通路・低い天井・しゃがみ・jump・sprint・水・Creative飛行、
3種類のmodifier演算を含む。原版server JARの数値tickと既存の自作LocalPlayer入力portであり、
グラフィカルclientのpacket時計やfull native dimension callback lifetimeの証明ではない。

同じ固定common-client binaryの公式1.16.1 / 1.21.11実接続では6項目・29項目を通過した。
元属性packet/ordinal、6段階の壁衝突としゃがみ、Creative/Spectator飛行、実modifier、
範囲外の開始拒否、変更と復元後の終端停止、停止・明示再開、実死亡/respawnによるworld寿命と
古いIDの拒否を検査する。送信した座標packetのbytesと予測frameを照合し、独立RCONのPosも保存する。
observer trace errorは0、実行中の予期しない位置補正は0。

source/build/binary/JAR・report・raw packet・RCONのhash、失敗した検証driverの実行も
[検証記録](evidence/common-player-scale-20261012.json)に保持する。
再現: `run_player_scale_oracle.py` / `run_player_scale_live.py`。
