# 共通の乗車関係

`Client::vehicle_state()`は両版で同じ`VehicleObservation`を返す。
own playerのJOIN／LOGIN identityと、実際のSET_PASSENGERSの順序付きID listを使う。
接続直後や無関係な車両のpacketしかない時、`relation`はNone。未受信を下車扱いにしない。

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(client: &Client) -> Result<()> {
    let observation = client.vehicle_state().await?;
    if let Some(relation) = observation.relation {
        match relation.value {
            VehicleRelation::Mounted { mount } => {
                println!("vehicle={}, source={}", mount.native_vehicle_id(), mount.receive_sequence());
            }
            VehicleRelation::Unmounted { previous_mount } => {
                println!("left vehicle={}", previous_mount.native_vehicle_id());
            }
        }
    }
    Ok(())
}
```

自分がpassenger listに含まれた時だけ`Mounted`となる。
その車両の後続listから自分が外れた時だけ`Unmounted`となり、元の`MountId`を保持する。
別の車両の空listは、現在の乗車関係を上書きしない。
`relation.source`と`passengers.source`は同じ元packet ordinal。capture boundaryは別の値で、
経過時間や無関係な受信から新しい関係を作らない。

`MountId`は接続・world・own player・native vehicle・元受信ordinalに束縛する。
乗車時点でspawnを受信している場合だけ、その元`EntityId`も保持する。
未受信のspawnを後から補わず、削除／ID再利用／world変更では関係を未知へ戻す。
despawn自体を「下車が実受信された」と読み替えない。保存した診断値からlive IDを復元しない。

乗車時にdry groundのmotion admissionを無効化する。後続の空passenger listやゼロvelocityだけでは
既定の立位pose・地上支持・motion条件を確認できないため、地上操作の許可は戻さない。
既存modernのinterruption契約を維持し、legacyの地上preview・target・採掘・設置・container open・
scene captureにも同じ受信乗車guardを適用する。fresh world baselineまでの制約は明示する。

この段階は関係の観測。明示的なowned下車送信と結果record、実乗車→下車の両版検証は
A5の残作業であり、A5完了とは扱わない。車両の現在位置・乗員の一般entity state・操縦／boat paddle・
physicsや下車後の広いmotion継続はBで統合する。
