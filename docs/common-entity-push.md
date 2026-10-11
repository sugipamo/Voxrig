# 明示的に選んだ2体の押し合いモデル

`model_living_pair_push(version, &input)`は、原版の
`LivingEntity.push(other)`を1回呼んだときの速度計算を共通APIで提供する。
player・cow・pig・zombieが継承する処理を対象とし、任意のentityの独自overrideは対象外。
通信や受信状態の変更は行わない。計算量・入力・出力は固定サイズ。

`LivingPairPushInput`は利用側のシミュレーション入力であり、受信値を現在の接触へ変換するAPIではない。
対象の検索、接触判定、補間時刻、同じtickでの対象順序・呼出し回数は利用側が決める。
`derived_geometry()`の箱やpacket targetだけから、その判断に必要な現在状態を推定しない。

| 入力 | 意味 |
| --- | --- |
| `first` / `second`の`position`・`velocity` | 呼出し時点のモデル位置とモデル速度 |
| `no_physics` | 原版の当たり判定を無効化する状態 |
| `vehicle` | 原版`isVehicle`の結果。乗員を持つ側で、乗っている側を表す値ではない |
| `pushable` | 原版`isPushable`の結果。生存・登れるblock・modernのSpectator判定は利用側で評価 |
| `first_sleeping` | 呼出される側の原版`isSleeping`の結果。2体の順序は入れ替え可能とは限らない |
| `same_vehicle` | 入れ子の搭乗を含む、原版`isPassengerOfSameVehicle`の結果 |
| `legacy_pushthrough` | 1.16.1の呼出される側のfloat値。1.21.11では0を指定する |

睡眠、同じ乗り物、どちらかの`noPhysics`の順で早期returnする。
水平距離の最大成分が原版の`0.01f`より小さければ速度を書き換えない。
それ以外は原版の平方根・正規化・`0.05f`の倍率を同じ順序で計算する。
Yの位置差は使わず、適用時のY速度は原版どおり`+0`する。

1.16.1は平方根をfloatへ丸め、`1f - pushthrough`をfloatで計算してから掛ける。
1.21.11は平方根がdoubleで、各bodyが`pushable`の場合だけ速度へ加える。
両版とも、乗員を持つbodyには加えない。`first_pushed`・`second_pushed`は
その1回の呼出しが速度加算を行うかどうかであり、実サーバーでの移動・接触・受付の確認ではない。

非有限の入力・計算結果は`NonFiniteState`、modern版に非ゼロのlegacy値を渡すと
`UnsupportedPushthrough`になる。対象のentity寿命や送信権限を入力・結果に含めない。
保存した数値モデルは元のspawn削除後も数値モデルとして使えるが、操作対象を復元できない。
同じ入力からの計算を繰り返すと同じ結果になる。結果の速度を次の入力へ渡すと、もう1回の押しになる。
受信イベントの重複排除やペアの実行順序をこの関数で代行しない。

## 検証

`scripts/run_entity_push_oracle.py`は、検証済みの公式JARを変更せず、登録済みの
原版ワールド内にテスト個体を作り、原版の押し合いメソッドを呼ぶ。
未接続playerへの通信を避けるため、ハーネスで搭乗関係とモードの入力状態を準備し、
原版のpredicateで読み直す。搭乗操作自体の検証ではない。
原版1,040ケース（legacy 560、modern 480）の速度と加算フラグを一致確認した。
各ケースの睡眠・搭乗・無効化・pushability条件が実際に成立することも確認する。
比較fixtureは`data/client_api/entity_push_oracle.json.gz`で、入力と結果のdoubleビット、
速度加算フラグ、原版JAR・mapping・ハーネスのhashを保持する。

`scripts/run_entity_push_live.py`は、変更していない公式両版へ2クライアントを接続する。
poseの実受信・解除・削除と、元のspawn packetの位置・受信順序を照合し、
その履歴を明示的なモデルの初期値として使う。predicateと速度は別途指定したモデル入力。
遠く離れたペアも利用側が明示的に選ぶため、この検証は自動の接触探索や
原版サーバーが実際に押し合ったことの検証ではない。
睡眠、同じ乗り物、無効化、順序、再計算、次のモデルstep、削除後の数値モデルを検査する。
公式両版の29項目（legacy 14、modern 15）を実接続で確認した。
[検証記録](evidence/common-entity-push-20261011.json)に結果とSDK・原版・各成果物のhashを保持する。
元のパケット・NBT・JAR・classはignored `.local`に残し、解釈済み結果とhashだけを公開する。

補間・自動の対象選択・移動制御への統合、team・cramming・独自の乗り物push、
自分のscale付き物理・elytra・追加の乗り物は引き続き#44の残件。
