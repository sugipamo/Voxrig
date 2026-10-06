# 共通の乾いた階段・ハーフブロック

共通`survival()`／`creative()`の有限地上歩行は、既存のpassive full cubesに加え、
選択した版の元registryで`SlabBlock`／`StairBlock`の実classとして登録された乾いた状態を扱う。
利用側は`preview_path`／`start_predicted_path`をそのまま使用する。
接続時以外に版別の操作を選ぶ必要はない。経路の選択は利用側に残す。

受信した名前と全propertiesを元の登録状態へ一致させ、collision・outline・interaction shapeを選ぶ。
名前のsuffix、欠けたproperty、別版の数値IDから形状を補わない。
stairsの向き・上下・内外の曲がり方とslabの上下・doubleは実受信値を使う。
隣接blockからstairsの`shape`を再推定しない。

同じ形状は立位の開始／終端検査、legacyのblock targetingと共通Survival sceneへ接続する。
normal-size standing、健康、通常のmovement attributes、effects・flight・未解決操作等の既存guardは維持する。
動いたという予測、完全な位置送信、実受信poseは別の情報である。

waterlogged、未知のsubclass、fluid、未対応block、moving shapeは拒否する。
各登録blockの元friction/speed/jump factorが通常値であることも取得時に検査する。
水・氷等の特殊物理、別姿勢、道具／effects、Creativeの飛行から立位への移行、一般採掘・設置の拡張は
この対応には含めず、ロードマップBの残作業とする。

## 確認方法

[元形状の出典](../data/client_api/dry_terrain_source.json)は公式server JAR、mapping、
classpathと独自observerのhashを保持する。元game methodは置換・コピーせず、直接呼び出す。
1.16.1は76 block／1,560 dry states、1.21.11は112 block／2,334 dry statesを取得した。
いずれも27種類のcollision／clip形状を区別する。

各版810入力のうち654の開始bodyが重ならないcollision結果を元のcombined VoxelShapeと比較する。
156の開始bodyが重なる入力も元結果とともに保持し、元`Shapes.joinIsNotEmpty`の判定を照合する。
bodyがsolid内にある状態は両adapterの立位検査で拒否するため、その状態でのbox分解collisionを
同等とは扱わない。元VoxelShapeの1e-7の境界判定も検査に使う。
outline／interaction shapeのray clipは各版810入力を元`BlockGetter.clip`と比較する。

実サーバーの`dry-terrain`シナリオは同じconsumerで、底slabからfull cubeとstairsを上り、
44tickの有限入力後にraised platform上でチェストを開閉する。各版のSurvivalとCreativeで
一つのClientを保ち、Survivalではcaptured sceneの同じ経路予測も照合する。
初期fixtureを整えた後はRCONを読み取りだけに使い、サーバー側の終点と元位置・input packetを確認する。
サーバーが受理した終点の確認は、元clientの全tick物理を再現した証明とは分ける。
結果は[実サーバー検証](common-client-native-validation.md)へ記録する。
