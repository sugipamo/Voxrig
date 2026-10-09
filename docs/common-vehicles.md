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

legacyの自動地上physicsも乗車受信後に止め、未完了の共通地上runは失敗履歴へ移して
後続frameを送らない。下車の完了だけでは地上支持や立位の許可を戻さない。
下車後の通常地上操作は、freshなown poseと既知の乾いた支持を検査する
`resume_ground(record.id)`へ接続した。[地上継続の契約](common-dismount-grounding.md)を参照。
車両の現在位置は`entity_motion`で観測する。乗員の一般entity stateと、下車後の広いmotion条件には引き続き制約がある。実サーバーの検証範囲は[検証記録](common-client-native-validation.md)に記載する。


## 有限の乗車入力

`survival().start_vehicle_control(mount, &inputs)`と
`creative().start_vehicle_control(mount, &inputs)`は同じAPIで、連続した実乗車に
有限のdigital入力を送る。`VehicleInput`は前後・左右をそれぞれ-1/0/1、jumpをboolで指定する。
1〜120入力まで、最後は必ず`VehicleInput::default()`のneutralとする。
各入力の間に50ms待つが、サーバーのtick数・処理回数や車両の移動量は保証しない。
sneakはこの入力に含めず、上記の明示的な下車を使う。

```rust,no_run
use voxrig::client::prelude::*;
async fn drive(client: &Client, mount: MountId) -> Result<()> {
    let inputs = [
        VehicleInput { forward: 1, ..Default::default() },
        VehicleInput::default(),
    ];
    let sent = client.survival().start_vehicle_control(mount, &inputs).await?;
    assert_eq!(sent.stage, VehicleControlStage::Submitted);
    Ok(())
}
```

`VehicleControlRecord`は送信前に保持する。呼び出しfutureの取消は待機だけを取り消し、
所有taskが有限の計画と最後のneutralを送る。`Client::vehicle_control_record()`はwriter待ち・
遮断中にも読める。`attempted_tick`はI/O前のframe意図、`dispatched_ticks`は完全送信した
frame数で、サーバーからの確認ではない。`Submitted`も移動・停止・制御ACKを意味しない。

途中の下車、同じ数値IDへの再乗車、mode/world/健康状態の変化、遮断や不確かなwriteは、
最初の理由を`RequiresInspection`へ保持する。後続入力やneutralを別の乗車へ送らず、
自動再送しない。未解決run中の別操作も拒否する。完全送信後は同じ実乗車に新しい有限runを
開始できるが、下車済みの`MountId`で再開できない。

実乗車を受信すると、完了候補だった地上runの記録を残して実行許可を退役させる。
乗車に伴う位置受信が先に届いて無効化されたrunも、全frame送信と最終予測restを
確認できる場合だけ退役させ、既存の失敗理由を保持する。実行中・途中送信は強制解放しない。乗車入力と下車の完了だけでは
地上のvelocity・支持・立位を補完しない。通常地上継続には、別の
`resume_ground(dismount_id)`で受信pose・宣言した停止値・2tickの履歴を保持する。
両版の元codecに全18入力ずつを照合し、実サーバーでは短い地上移動→乗車→
有限入力→neutral→実下車→切断を両modeで検証する。
車両付近の乾いたrail地形は[共通地形](common-dry-terrain.md#車両付近の乾いたrail)、
車両の受信位置観測は下記へ接続した。通常のboat操縦とpaddleは下記へ接続した。乗り物全般のphysicsには引き続き制約がある。


## 乗車中の受信motion

`mount.vehicle()`で取得した元spawnを`client.entity_motion(target).await?`へ渡すと、
後続の位置target・body/head回転・速度sample・ground flagを読むことができる。
それぞれの受信ordinalと、変化させないspawn履歴を保持する。
両版・両modeで有限乗車入力による位置更新を観測し、独立RCONの移動と照合した。
[観測契約と未解決の相対補正](common-entity-motion.md)を参照する。
車両physics・停止・下車後の地上操作許可は、この観測から補完しない。


## ボートとトロッコ（2026-10-08）

同じ`start_vehicle_control`で通常のボートを操縦できる。最初の実乗員であることを確認し、
各tickで元の入力・パドル・車両位置を送る。水面の浮力、前進・後退・旋回、惰性、
空中から水面への移行、通常のブロック衝突と陸上の摩擦を版ごとに計算する。
`boat_motion.received`は受信した元のentity sample、`initial_frame`と`frames`は予測であり、
受信位置やサーバーの承認を合成しない。最初の角速度は明示的な0のseedとする。
同じ連続乗車の完全送信済みrunからは、最後の予測を引き継ぐ。
`attempted_frame`は3パケットのI/O前に残す。3つすべての送信後だけ`frames`へ追加する。

未読込の地形、ボートに対する泡の柱、溶岩、スライム・蜂蜜・クモの巣などの未実装hookは、
そのtickの送信前に拒否する。他entityとの衝突や乗り物固有の効果全般は再現していない。
検証済みの範囲は単独の通常ボートと水面・水没・流水・空中・通常の陸上地形。
操作中にサーバーから車両位置補正や爆発を受けた場合、同じ乗車が続いていても
`RequiresInspection`へ移し、後続tickを送らない。乗員順序が変わり操縦席から外れた場合も中断する。
`vehicle_state().motion_correction_sequence`はその受信境界であり、補正位置そのものではない。

最後のneutralはパドルを解除する。速度を瞬時に0にはせず、惰性が残る。
トロッコは元の入力をサーバーへ送り、サーバー側のレールとphysicsに任せる。
停止には無給電のパワードレールなどの制動条件が必要。`Submitted`を停止確認として使わない。

公式の両版で17場面ずつ、合計1,540tickの位置・速度・回転・角速度・接地・水面接触・パドルを
元の処理と完全一致で比較した。再生成手順は[移動oracle](movement-oracle.md#ボートの比較2026-10-08)。

2026-10-09、水没と流水の操作を追加した。水源への水没は減速係数0.45と微小な浮力、
流水への水没は減速係数0.9と重力0.0007を使う。通常の水面への浮力とは区別する。
水流の押しは`Entity.baseTick`の処理を使い、平均した流れを非player用に正規化する。
空中から水へ入る際の水面への移行は、1.21.11では移動先の衝突確認も行う。
`BoatFrame::in_water`は水面または水没を示し、受信した水没情報ではない。

サーバーは連続して水没と判定した約60tick後に乗員を降ろす。SDKは下車を予測で確定せず、
実際に受信した乗員関係の変化で`RequiresInspection`へ移し、後続入力を止める。
新しい[液体操作oracle](movement-oracle.md#泡の柱とボートの液体操作2026-10-09)は、
従来分も含め33場面×2版、計2,500tickをbit単位で比較する。
実接続では水没中の旋回・入力解除・下車、入力なしでの水流による移動と、
強制下車の受信後に車両位置・パドルを送り続けないことを確認する。

```bash
cargo build --locked --example climbing_control_probe
python3 -B scripts/run_fluid_control.py --accept-eula \
  --jars /absolute/path/to/downloads \
  --binary "$PWD/target/debug/examples/climbing_control_probe"
```

同じ実接続検証でplayerの泡の柱の上下・水面脱出・継続操作の停止も確認する。
ボートへの泡の柱の効果は引き続き範囲外である。

実接続の再検証には次を使う（JDK 21、ビルド済みprobe、公式JARが必要）。

```bash
cargo build --locked --example climbing_control_probe
python3 scripts/run_climbing_control.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe \
  --jars /absolute/path/to/downloads --vehicle boat --vehicle-mode survival
```

`--vehicle minecart`、`--vehicle-mode creative`も指定できる。両版へ接続し、
元の乗車受信、前進、neutral後の停止、実際の下車と古い乗車IDの拒否を検査する。
ボートでは旋回・後退と全tickの実パケットも検査する。
RCONで独立に位置・停止・乗車関係を確認し、proxyは元のバイトをそのまま転送する。
結果とパケット記録は`.local/climbing/live/*/report.json`へ残す。
両版・両modeの通過結果と座標・元reportのhashは
[検証記録](evidence/climbing-vehicle-control-20261008.json)へ保存した。
