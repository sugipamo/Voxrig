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
同じvehicle・spawnのlistが更新されても、自分が引き続き乗っていれば元の`MountId`を保持する。
他の乗員の増減で自分の連続した乗車寿命を更新せず、観測sourceだけを新しいpacketへ進める。
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

## 明示的な下車

`survival().dismount(mount)`と`creative().dismount(mount)`は、実際に受信した乗車寿命に対して
下車入力を一度送る。同じmode・接続・world・生存状態を送信直前にも確認する。
`DismountRecord`は送信前に保存され、呼び出しfutureを中断してもactorが元の要求を所有する。
同じmountや未解決のattemptを再送しない。読み取りは`Client::dismount_record()`から行い、
writer待ちや切断後にも記録を読める。

順序は次のとおり。

1. 実際の`Mounted { mount }`から`dismount(mount)`を呼ぶ。
2. 全入力frameの送信は`Submitted`。サーバーの結果確認とは別の値。
3. 元の車両の後続passenger listから自分が外れた実受信で`ObservedUnmounted`となる。
4. 同じhandleから`complete_dismount(record.id)`を呼び、neutral入力を一度送る。
5. 実除外とneutralの完全送信が揃うと`Completed`。重複した解除を拒否する。

```rust,no_run
use voxrig::client::prelude::*;
async fn request(client: &Client, mount: MountId) -> Result<DismountId> {
    let record = client.survival().dismount(mount).await?;
    Ok(record.id)
}
async fn finish_when_received(client: &Client, id: DismountId) -> Result<bool> {
    if client.dismount_record().await?.is_some_and(|r| {
        r.id == id && r.stage == DismountStage::ObservedUnmounted
    }) {
        client.survival().complete_dismount(id).await?;
        return Ok(true);
    }
    Ok(false)
}
```

1.16.1はPLAYER_INPUTのneutral axesとshift flag、1.21.11はInputのshift bitを使う。
要求直後にneutralを連続送信しない。元サーバーはtick時のshiftを見て下車するため、処理前の解除は
要求を取り消し得る。経過時間・velocity・despawnを実除外の代わりに使わない。
別接続／world、mode変更、死亡、関係の消失・別車両への乗車、部分送信などは
`RequiresInspection`を保持し、自動retryしない。実除外は観測事実であり、要求との因果ACKではない。

legacyの自動地上physicsも乗車受信後に止める。下車の完了だけでは地上支持や立位の許可を戻さない。
車両の現在位置・乗員の一般entity state・操縦／boat paddle・physicsや下車後の広いmotion継続は
Bで統合する。実サーバーの検証範囲は[検証記録](common-client-native-validation.md)に記載する。
