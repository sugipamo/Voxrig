# 共通の継続操作（P7）

1.16.1のbest-effort接触モデル破棄は通常の`ControlRecord.degradation`と
`control_changes_after`へ届く。[劣化通知の契約](control-degradation.md)と
[移行方法](migration-control-degradation.md)を参照。

地上・held flightの継続モデルは、1.21.11の自分のplayerの実受信scaleを
0.125〜4の範囲で扱う。寸法・目の高さは原版のfloat演算で姿勢とscaleから作り、
段差の高さや移動・ジャンプ速度に独自の倍率を掛けない。
`ControlFrame.model_scale`・`bounds`・`eye_position`は数値予測で、受信確定値ではない。
元属性・modifier・receive ordinalは`player().attributes`で別に観測する。

操作中に開始点と異なるscaleを受信すると、対応範囲内でもownerを終了して入力を解除する。
`ControlRecord.scale_change_sequence`はその最初の実受信ordinalを保持する。
同じ値の再受信は終了条件にならず、変更後に元の値へ戻っても旧ownerを再開しない。
新しいownerを明示的に開始する。欠測・非finite・範囲外は送信前に拒否する。
原版のconstructorからの既知のunit初期値と、実際に受信した属性のsourceは区別する。
1.16.1にはscale属性がなく、寸法は1、変更sequenceは`None`。

[scale操作の契約と検証](common-player-scale.md)を参照。
単発の採掘・到達距離・有限pathや乗車操作のscale制限は、この拡張の対象に含めない。

`Client::survival()`の`start_control`・`set_controls_for`・`request_ground_jump`・`stop_control_for`・`control_record`は、
キーを押し続ける操作を1.16.1と1.21.11で同じ型で扱う（`voxrig::client::control`）。

```rust,no_run
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
let session = survival.start_control().await?;   // 現在のSDKモデルと向きで、キーを離した状態で開始
survival.set_controls_for(session.session_id, Controls { forward: 1, sprint: true, ..Default::default() }).await?;
tokio::time::sleep(std::time::Duration::from_secs(2)).await;
let record = survival.control_record().await?;   // 予測した位置・状態、補正の回数
survival.stop_control_for(session.session_id).await?; // 対象sessionのダッシュとしゃがみを離して終了
# let _ = record;
# Ok(())
# }
```

## モードを指定する地上操作

`Client::player_control(GameMode::Adventure)`と`GameMode::Creative`にも、
`start_control`・`set_controls_for(id, controls)`・`request_ground_jump(id)`・
`stop_control_for(id)`・`control_record`がある。同じ入口でSurvivalも扱える。
モードは受信値と照合し、handleを作っても変更しない。Creativeも飛行を無効にした地上モデルである。
Spectatorの地上開始は`Unsupported`、飛行中や乗車中の開始は拒否する。

`ControlRecord::mode`は開始時に照合したモード。
`mode_change_sequence`は、そのownerと異なるモードを実際に受信した最初のordinalである。
1 tickの間に元のモードへ戻っても旧ownerは停止し、新規開始まで送信しない。
`flight_change_sequence`も実際のabilitiesの飛行開始を保持し、地上へ戻っても旧ownerを再開しない。
別モードのhandleと古いIDによる入力変更・ジャンプ・停止は他のownerを変更しない。
停止済み記録の読取りと一致するownerの終了確認は、モード変更後にもできる。

```rust,no_run
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;
# async fn walk(client: &Client) -> Result<()> {
let ground = client.player_control(GameMode::Adventure);
let owner = ground.start_control().await?;
ground.set_controls_for(owner.session_id, Controls { forward: 1, yaw: 37.0, ..Default::default() }).await?;
ground.request_ground_jump(owner.session_id).await?;
ground.stop_control_for(owner.session_id).await?;
# Ok(())
# }
```

両版の公式サーバーで各27項目、計54項目を検証した。
[結果・ハッシュ](evidence/common-ground-modes-20261011.json)と`run_ground_modes.py`を参照。
元packet／NBTは非公開のローカル記録に保持する。

## Creative・Spectatorの連続飛行

`client.player_control(GameMode::Creative | GameMode::Spectator)`の
`start_flight_control(Controls)`は、最初のtickから適用するキーを明示して飛行モデルを開始する。
Creativeは先に既存の`client.creative().set_flying(true)`で飛行を有効にする。
Spectatorは原版から受信した飛行状態を使う。現在worldの原版abilitiesと飛行速度が必要で、
未受信の速度を0.05で補わない。速度0は原版どおり受け付ける。

```rust,no_run
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;
# async fn fly(client: &Client) -> Result<()> {
client.creative().set_flying(true).await?;
let flight = client.player_control(GameMode::Creative);
let owner = flight.start_flight_control(Controls { jump: true, ..Default::default() }).await?;
flight.set_controls_for(owner.session_id, Controls { forward: 1, sprint: true, yaw: 37.0, ..Default::default() }).await?;
flight.stop_control_for(owner.session_id).await?;
# Ok(())
# }
```

ジャンプは上昇、しゃがみは下降となり、地上ジャンプのqueueは受け付けない。
ダッシュと原版の飛行速度で横方向の加速を計算する。水中でも泳ぎへ切り替わらない。
Creativeはブロックに衝突し、Spectatorは原版のnoPhysicsでブロックを通過する。
旧版の箱中心による座標丸めと、最新版の直接座標加算をそれぞれ使う。

`ControlRecord.kind`は`ground`、`flight`または`glide`。`flight_parameters`は最後にモデルが使った
受信速度とそのordinalであり、現在のabilitiesやサーバーの承認を表さない。
`ControlFrame.flying`もモデルの結果である。
モード変更、飛行解除、権限喪失は旧ownerを終了し、同じtick内に復元されても再開しない。
停止は入力を解除するが、飛行の無効化や物理的な静止を意味しない。
再開は新しいIDと明示的な初期キーを使う。

Creativeがモデル上で着地したtickでは、原版clientの順序でabilitiesの解除、移動、入力解除を送る。
`flight_disable`は解除前の受信ordinalと完全書込みの有無を保持する。
移動や入力解除の書込みが失敗しても解除の結果を隠さず、旧ownerは再送しない。
着地は飛行ownerを終了するため、地上の継続は`start_control()`で明示する。
古い受信飛行フラグに対するローカル解除と、新しい原版abilities通知を区別する。
worldが変わった場合には旧worldの入力解除を送らない。

有限の`set_flying`・`move_flying`・`land`の意味はそのままで、このAPIに暗黙変換しない。
エリトラ、乗車、浮遊効果、未監査の地形・姿勢は別の対応範囲である。
原版比較は両版38シナリオ・1,424 tick（離陸、速度0、昇降、ダッシュ、壁、水、着地、
クモの巣、Spectatorのハチミツ通過）で行う。
両版の公式サーバーで計70項目（1.16.1: 34、1.21.11: 36）、予期しない補正0件を確認した。
[結果・ハッシュ](evidence/common-held-flight-20261011.json)を参照。
最新版の高速ダッシュとSpectator壁通過は、明示停止後の最後の送信frameをRCONで観測する。
速度や位置誤差は変えず、慣性を保持した再開は新しいIDで明示する。
高速で連続送信しながらRCONが全snapshotを観測できるという主張はしない。
実接続の再現には`run_flight_control.py`とビルド済み`climbing_control_probe`を使う。
原packet/NBTは`.local`に保持し、公開証跡は結果とハッシュだけにする。

## ハチミツの壁（1.21.11）

Survival／Adventure／Creativeの地上held controlは、落下中にハチミツの側面へ
触れると原版の水平減速・鉛直速度制御・落下距離リセットを行う。
接触には姿勢と受信scaleによる幅を使い、移動軸ごとのcallback順と重複除去を保つ。
しゃがみ中も滑降し、壁を離れると通常の落下へ戻る。上面の速度／jump係数と
ボートの非Living処理は従来の別処理を使う。有限pathの地形制限は変わらない。
1.16.1の壁滑降、未ロード地形、既存の不確かな接触の拒否条件は維持する。
数値の根拠と実接続の再現手順は[movement oracle](movement-oracle.md#ハチミツ壁の滑降2026-10-23)を参照。

## 浮遊効果（1.21.11 Ground）

Survival／Adventure／Creativeの地上held controlは、受信した浮遊効果のamplifier 0〜255を
非公開の物理環境へ渡す。操作中の付与・amplifier変更・解除を次のモデルtickへ反映し、
天井接触後も水平入力とtick送信を続ける。複数のeffect packetがtick前に届いたときは
受信順で更新した最後の状態を使う。受信durationをSDKの時計で減らして解除を推測しない。

効果の有無だけからamplifier 0を補わない。不正packetは既存decoderで拒否し、
world変更時は旧worldの効果を破棄して旧ownerを終了する。元の効果とordinalは従来の
`player_state().effects`で読み、モデルの速度や送信成功と区別する。
低速落下との併存、水・溶岩の別travelも原版oracleで照合する。
飛行・Glideの浮遊拒否と有限dry previewの制限は維持する。1.16.1の浮遊は未資格。
[原版比較・実接続手順](movement-oracle.md#浮遊効果中のground操作2026-10-23)を参照。

## 動き

- sessionを記録する呼び手は、`set_controls_for(id, controls)`と`stop_control_for(id)`で対象を指定する。
  SDKが状態ロック内でIDとworldを照合するため、観測後に別sessionへ入れ替わっていてもその入力を変更・解除しない。
  IDは接続内の識別子であり、権限を与えるtokenではない。
  `set_controls`と`stop_control`も残るが、その時点のsessionを選ぶ操作である。
  前worldのsessionを停止するときは、停止記録だけを返し、新worldへ解除パケットを送らない。

- clientが50 msごと（clientの時計）に[共有物理エンジン](physics-engine.md)を1 tick進め、
  そのtickにclientが送るものを送る。押したキーは`set_controls`で置き換えるまで有効で、次のtickから効く。
  - 1.21.11: 入力パケット（前後左右・ジャンプ・しゃがみ・ダッシュのキー。変化したときだけ）、
    ダッシュの開始・停止コマンド（変化したとき）、位置・向き・接地・横の衝突。
  - 1.16.1: ダッシュの開始・停止、しゃがみの押下・解除（変化したとき）、位置・向き・接地。
  - ダッシュは、キーを押していて公式clientの開始条件を満たすときに始まり、停止条件で止まる（ダブルタップは使わない）。
- 送った位置は**送信であってserverの受理ではない**。`ControlRecord::frame`は予測値。
- `ControlFrame::eye_in_water`は、予測した姿勢での目が水中かどうか（公式の`updateFluidOnEyes`）。
- 受信したものは公式clientと同じように取り込み、続ける。
  - serverからの位置（テレポート・補正）: 位置を置き換える（`corrections`に数える）。
  - 自分への速度（ノックバック等）: 速度を置き換える（`velocity_updates`に数える）。
  - 1.21.11のGround ownerは[爆発のoptional impulse](common-explosions.md)を受信順に一度だけ加算する。
    正常な明示停止後の同じモデルも新しいimpulseを取り込み、再開時に古い加算を繰り返さない。
    Flight／Glide／vehicle、未decode・world変更・不確かなwriteの中断は維持する。
    補正による既存の接触origin失効はownerを停止するため、その場合は新ownerが必要。
  - 開始時は現在のSDKモデルとローカルのyaw/pitchを使い、移動・ジャンプ・しゃがみ・ダッシュの入力を離す。
    同じworldでの停止・再開でも、すでにモデルへ取り込んだ古いノックバックを再適用しない。
    停止中に届いた新しい位置・速度は受信順に取り込み、古いノックバックで新しいテレポートを上書きしない。
    開始境界までの受信は初期状態に含み、補正・速度更新のカウンタは開始後の更新を数える。
  - 移動速度の属性（受信した修飾子にclient自身のダッシュ修飾子を加える）、効果、満腹度、飛行許可、ネザー（溶岩の流れ）。
- 予測できないtick（範囲外の地形、未ロードのchunk）では`Paused`になり、そのtickは何も送らない。
  毎tick同じ状態からやり直し、予測できれば`Running`に戻る。
- 1.16.1は停止中も進む互換モデルの現在の位置・速度・接地・落下距離から再開する。
  互換モデルがまだ進んでいなければ共通エンジンの状態全体を引き継ぎ、進んだ後は古い水中・姿勢状態を再利用しない。
- 1.21.11は同じworldの自分の位置受信が必要。位置送信が未解決、速度の受信基準が不明、
  または他の操作による位置送信の後に新しい位置受信がない場合は開始を拒否する。
  最初の再開パケット、繰り返し再開、落下と着地、停止後の位置受信を両版の公式serverで追加検証し、
  元のbytes・SDK revision・binary/JAR hashを[レビュー検証記録](evidence/common-control-restart-review-20261009.json)に保存した。
  `scripts/run_control_restart.py --accept-eula`で再実行できる。
- 次の場合は`Stopped`になる: `stop_control`、切断、world（死亡からの復帰・次元）の変化、game modeの変化、飛行、乗車、
  未資格の爆発などエンジンの外の動き、死亡、送信の失敗。止まったsessionは再開しない（`start_control`で新しく始める）。
- 同じworldで接続が使える場合、自動停止でもダッシュ・しゃがみ（1.21.11では全入力）を解除する。
  `stop_control`が対象sessionを取得した後は、待ち手がキャンセルされてもClientのtaskが入力解除と記録保持を完了する。
- sessionの間は、静止を前提にする操作（移動の予測・有限の移動・照準・採掘・設置・収納を開く）は拒否する。
  1.16.1では、sessionの間`Bot`自身の物理と`set_control`のloopを止める（開始時に`Bot`のキーが離されていることを要求する）。
  停止中はSDKの通常の物理loopが進むため、再開はその時点のモデル位置・速度から始まる。
  1.21.11では停止した共通sessionのモデルを引き継ぐ。停止後に別の有限移動・位置送信を行った場合、
  新しい位置受信がない限り、そのsessionの古いモデルを使った再開は拒否する。
  同じ座標に戻っていても、別の移動送信を現在のモデル速度の証拠として扱わない。

## 単発の接地ジャンプ

`request_ground_jump(session_id)`は、指定した現在のsessionの**次の物理tickに一度だけ**接地ジャンプを予約する。
`start_control`が返した`ControlRecord::session_id`を使う。入力と視線は変更しない。

```rust,no_run
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;
# async fn jump(client: &Client) -> Result<()> {
let survival = client.survival();
let session = survival.start_control().await?;
survival.set_controls_for(session.session_id, Controls { forward: 1, sneak: true, yaw: 37.0, ..Default::default() }).await?;
// 呼出側で接地したframeを観測してから、そのsessionに一度要求する。
let queued = survival.request_ground_jump(session.session_id).await?;
let latest = survival.control_record().await?.unwrap().ground_jump;
# let _ = (queued, latest);
# survival.stop_control_for(session.session_id).await?;
# Ok(())
# }
```

- 同じtickを待つ予約は同じrequest IDにまとめる。呼出しの成功は`Queued`というローカル受付であり、移動送信でもserverのジャンプ確認でもない。
- 接地はそのtickのclientモデルで判定する。`Applied`は版固有の接地ジャンプ関数の予測で、jump factor・効果・属性・ダッシュを通常の物理から取り込む。
  接地flagがfalse（開始直後の未確定も含む）なら`Airborne`、接地していても深い水・溶岩なら`InFluid`、modernのジャンプ属性が無効なら`NoJumpPower`として消費する。着地後に再試行しない。
- `Controls::jump`をすでに押している場合は`AlreadyHeld`として追加の力を加えず、通常のジャンプ・泳ぎ・登りをそのまま続ける。
  押していないキーを継続的に押すことも、他の呼出しが押しているキーを離すこともない。
- 1.21.11では、`Applied`を予測したtickだけ送信inputのjump bitを立て、次のtickは保持中の入力に戻す。
  `ControlRecord::controls`は常に呼出側が保持しているキーを示し、この一時的な送信bitとは別。
- `Evaluated { tick, outcome, dispatched }`は一度消費した結果。`dispatched`はそのtickの移動を完全に書いたときだけtrueになり、server受理を表さない。
  latest requestは`control_record().ground_jump`から読み、送信失敗後も結果とsessionの停止理由を参照できる。
  `dispatched=false`でも、一部のpacketが送られた可能性はある。停止理由と合わせて観測し直す。
- 存在しない・古い・停止したsession、Survival以外の受信mode、別world、飛行・乗車・死亡・遮断した接続では受付を拒否し、入力を変更しない。
  1.16.1のnative `Bot::jump()`も共通の移動所有者の間は拒否し、nativeの未処理予約がある間は共通sessionの開始を拒否する。
  互換APIの予約を停止後のジャンプとして持ち越さない。
- `Paused`中の新規要求は拒否する。予約後に次のtickを予測できなくなった場合も`Cancelled`とし、terrainの回復後へ持ち越さない。
  SDKの再構築による一時停止も同じ。停止・world/mode変更・切断・revocationまでに未評価なら停止記録は`Cancelled`を保持する。
- 受付lockを待つ間の呼出側キャンセルは予約を作らない。受付済みの予約はClientのtaskが処理するので、呼出側が戻り値を使わなくても消えない。
  停止の待ち手をキャンセルしても、既存の接続所有taskが入力解除と停止記録を完了する。
- 記録はlatest requestを保持する。一度評価後の次の要求で置き換わり、明示的な新session開始で初期化する。
  新sessionへ予約を移さない。必要な停止記録は`stop_control`の戻り値を保存する。

## 範囲と制限

- 地形・液体・効果の範囲は[エンジンの説明](physics-engine.md)のとおり。梯子・つる・足場を登れる。
  ジャンプを押すか、梯子・つるの壁へ進むと登り、しゃがむと梯子・つるで滑り落ちるのを止める。
  足場ではジャンプで登り、しゃがむと下へ降りる。梯子と向きが一致する上の開いたトラップドアも登れる。
  泡の柱の上昇・下降、水面からの出入りにも対応する。飛行・乗車は扱わない。
  泡の柱は両版の公式サーバーへ実接続し、上下・水面脱出・停止を
  [元の通信とRCON座標](evidence/common-fluid-control-20261009.json)で確認した。
- 1.16.1は受信したプレイヤースロット8の靴から深海探索者・ソウルスピードを環境へ反映する。
  エンチャントは元の`Enchantments`の最初の一致を使い、保存用`StoredEnchantments`は適用しない。
  深海探索者のレベルは公式と同じ0..255で読み、物理への効果は3で上限にする。
  ソウルスピードの対象ブロックは受信した`minecraft:soul_speed_blocks`タグを使う。
  タグが欠けている・解釈できない場合、または装備が`Unavailable`の場合、そのtickは送信前に`Paused`になる。
  タグの明示的な空集合を標準ブロックで補わない。サーバー側の速度補正は受信属性として取り込み、独自に加算しない。
- 1.21.11の水中移動効率は受信属性の加算・ベース乗算・総乗算を公式の順序で計算し、0..1に制限する。
  `ControlRecord::movement_equipment`は最後にモデルへ渡した変換済み入力を示す。
  停止後にも読める診断値であり、現在の装備・受信事実・操作権限・サーバー受理を表さない。
- アイテムの使用中（盾・弓・食事）の減速は、受信した`using_item`に従って再現する。sessionの間も`use_item`・
  `release_use_item`を送れる（[アイテム使用](common-item-use.md)）。
- 時刻はclientの時計で、serverのtickとは同期しない（公式clientと同じ）。

## 装備の移動効果の検証（2026-10-10）

公式の物理処理による11場面・330 tickで、深海探索者のレベル・最初の一致・保存用エンチャント、
ソウルスピード、modernの水中移動属性の全操作と上限を厳密比較した。
ソウルスピードの速度属性は公式`tryAddSoulSpeed`で初期入力に確立し、SDKが加算を作る比較にはしていない。
改変していない両版の公式サーバーへ共通Clientで接続し、計13項目が通過した。
装備更新中も同じ操作所有者を使い、元の属性・位置パケットとRCON位置を照合した。予期しない補正は0回。
[検証記録](evidence/common-equipment-movement-20261010.json)は結果と入力・SDK・binary・公式JARのhashを公開する。
元の通信・NBT記録はローカルに保持する。
既存のCreative明示飛行も同じSDKで両版の公式サーバーへ接続し、有効化・3回の空中移動・
一度の飛行解除とneutral・有限の着地・地上移動・収納操作への継続を再確認した。
これは継続操作エンジンの飛行・エリトラ対応を意味しない。

```sh
cargo build --locked --features native --example climbing_control_probe
python3 -B scripts/run_equipment_movement.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe --compiled-sdk-revision HEAD
```

## 単発接地ジャンプの実接続検証（2026-10-09）

公式1.16.1・1.21.11サーバーで共通APIだけを使い、計21項目が通過した。
前進・しゃがみ・照準を保持した1回のジャンプ、空中で消費して着地後に再試行しないこと、
押し続けている水中・梯子入力の保存、停止・一時停止・mode変更・respawn・session置換・
接続遮断後の拒否と結果保持を検査した。modernでは受信したジャンプ強度0も検査した。
予期しない補正は0回。元のmovement packet、modernの単発jump入力と解除、
RCONの位置、実行SDK revisionとsource・binary・公式JAR hashは
[検証記録](evidence/common-ground-jump-20261009.json)に保存した。
回帰テスト924件、doctest、strict Clippy・rustdocも通過した。

```sh
cargo build --locked --example climbing_control_probe
python3 -B scripts/run_ground_jump.py --accept-eula
```

## 停止・再開の検証（2026-10-09）

`run_control_resume.py` が公式 1.16.1・1.21.11 サーバーへ共通 Client で接続し、
歩行中・落下中の停止と再開、照準を保持した解除入力、着地、停止中に届いた新しい位置補正、
停止した記録の保持を確認する。再開後の元の movement packet と RCON 座標を保存する。
[検証記録](evidence/common-control-resume-20261009.json) に実 source・binary/JAR hash、
両版の計11確認と初回失敗を記録した。914テスト、strict Clippy、Rust 1.85 の全ターゲット確認も通過した。

1.16.1 の着地は10 block、1.21.11 は対応範囲内の4 blockの落下を使う。
1.21.11 の最初の10 block落下は、既存の `fall-distance reset sweep` 制限で `Paused` になった。
この試行を着地成功とは扱わず、最終fixtureでは10 block落下が同じ制限で送信を止め、
記録を保持することを別に確認する。物理の範囲外判定は変更していない。

```sh
cargo build --locked --features native --example climbing_control_probe
python3 -B scripts/run_control_resume.py --accept-eula
```

停止中のモデル進行は版ごとに上記の契約に従う。この確認は単発ジャンプ要求、装備交換、
消費側の共通移動API移行、保存worldの元事象再現を完了したことを示さない。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`（1.16.1・1.21.11）をoffline-modeでlocalhostに起動し、`examples/continuous_control_probe.rs`で確認した。
consoleで前方に2 blockの深さの水場を作り、同じ接続で次を続けて行った:
歩く20 tick → ダッシュ10 → ダッシュジャンプ12（水に飛び込む）→ 水底でしゃがんで歩く10 → 止まる10 →
ジャンプしながら泳いで渡り、岸に上がる80 → 止まる20 → 歩いている途中でconsoleからテレポート → 歩いて止まる。

| 版 | 送ったtick | serverの補正 | 最後の位置（予測） | serverの位置（`data get entity`） |
| --- | --- | --- | --- | --- |
| 1.21.11 | 約210 | 1（意図したテレポートのみ） | `[5.5, -60.0, 33.85794463056311]` | `[5.5d, -60.0d, 33.85794463056311d]` |
| 1.16.1 | 約210 | 1（意図したテレポートのみ） | `[232.5, 4.0, -190.35791443778544]` | `[232.5d, 4.0d, -190.35791443778544d]` |

テレポートの後もsessionは`Running`のまま続き、押したキーで歩き続けた。どちらの版も、serverが記録した最後の位置は予測とbit単位で一致した。

## 登りと停止の実接続検証（2026-10-08）

`examples/climbing_control_probe.rs`は共通APIだけを使う。`scripts/run_climbing_control.py`が
localhost限定の公式サーバーを版ごとに起動し、実際のblock更新を進めながら検証する。
RCONの`data get entity ClimbingProbe Pos`でserver側の座標を独立に取得し、
元の通信をそのまま転送するproxyで入力解除パケットを確認する。

両版で次が通過した:

- 高さ65から、梯子・つる・足場をジャンプで32 tick登る。
- 梯子・つるの壁へ20 tick前進して登り、12 tickしゃがんで高さを保つ。
- 足場で12 tickしゃがむと下へ降りる。
- 水浸しの梯子・足場を登り、梯子では壁へ前進して登る。水中の足場はしゃがんで沈む。
- 梯子と向きが一致する開いたトラップドアを通って登る。向きが異なるものと閉じたものは越えない。
- 停止すると入力解除を送り、以後の記録は変化しない。別sessionを開始できる。
- ダッシュ中の外部テレポートを1回だけ取り込み、`Running`のまま継続する。
- creativeへのmode変更では自動停止してダッシュと入力を解除する。
- 切断後も最終の`Stopped`記録を読める。

登り中の予期しない補正は両版とも0回。予測とRCON座標の比較は、独立した時計のずれを考慮して
最大0.4 blockの差まで許す。通信で実際に送った解除と、待ち手のキャンセル後の解除・記録保持を調べる回帰テストも追加した。

再実行（Java 21が必要。`--accept-eula`はMinecraft EULAへの同意）:

```sh
cargo build --locked --example climbing_control_probe
python3 -B scripts/run_climbing_control.py --accept-eula
```

サーバー・Client・パケットのログと`report.json`は`.local/climbing/live/`へ保存する。
水浸しとトラップドアを含む両版25項目ずつの通過結果と元reportのhashは
[検証記録](evidence/climbing-vehicle-control-20261008.json)へ保存した。
既存の公式jarを使う場合は`--jars /absolute/path/to/downloads`、別のCargo targetを使う場合は
`--binary /absolute/path/to/climbing_control_probe`を指定できる。jarのSHA-1を検証する。

## 生物接触の操作への統合

通常scaleのplayer / cow / pig / zombieの完全な受信履歴を操作ownerへ結び付ける。
[生物接触owner](common-entity-owners.md)と[特殊dispatchの範囲](common-entity-dispatch.md)を参照。
補正・履歴不足・未検証入力ではモデルを破棄し、古い起点から自動再開しない。
ボートの失敗時は同じ健康な実乗車の受信境界に限りneutral/paddle解除を一度だけ試み、
予測位置を再送しない。`failure_release`はintent・完全送信・拒否/不確実性を保持する。

## Elytraの滑空

`PlayerControl::start_glide_control(Controls)`は実装備・実flagを使う独立owner。
開始writeは受理ではなく、owner停止も物理的な滑空終了ではない。
操作・終端停止・未対応boostと証拠は[Elytra滑空](common-glide-control.md)を参照。

追加姿勢の有限な一覧と未対応のsleep/Riptide操作は[姿勢の対応範囲](common-posture-coverage.md)を参照。

## 接触を証明できない場合の best-effort 地上移動

1.21.11 の `body::Action::Move` は接触予測を best-effort で扱う。
両版の `Client::player_control(mode).start_control_best_effort()` でも明示的に選べる。
従来の `start_control()` は厳密な receipt/model API として残す。
1.16.1 の body Move 接続は後続の変更であり、今回は ground control の opt-in を使う。

近傍の未監査種・騎乗 group・接触履歴や遠隔地形の不足などで、接触モデルの作成、
更新または staged tick に失敗すると、その owner の遠隔接触モデル全体を破棄する。
同じ自分の身体状態から接触抜きで予測して送信し、その理由だけでは owner を停止しない。
部分的な接触状態や途中まで計算した tick を再利用せず、この owner では再構築しない。
`contact_diagnostic_for(session_id)` は `MovementRefusal::RemoteStateDropped` を返し、
遠隔の数値接触状態を捨て、同じ owner・移動入力を続けたことを明示する。
内側の `refusal` に元の原因・受信境界・geometry を保持する。
passenger history unavailable、lost relationship placement history、scope 再進入・再選択、
sweep 超過の `start a new control owner` は内側の厳格モデルの診断であり、
この opt-in owner を作り直す要求ではない。
その context の `NotAdmitted/Invalidated` は厳密な接触モデルの状態であり、
best-effort control owner の停止を意味しない。実際の状態は `ControlRecord.status` にある。
既存の公開 record に field を足さず、診断チャネルに additive な variant を追加した。
補正を受信した場合は、その pose を自分のモデルの再起点とする。
旧起点の遠隔数値状態が残っていれば破棄して同じ入力を続け、既に破棄済みなら元の警告を保つ。
共有の entity 観測・履歴は変更せず、この owner では遠隔モデルを再構築しない。

world/spawn/owner identity と自分の受信 mode・生存・乗車等の照合は維持する。
自分の移動を計算する地形や再構築の不足、在庫等の未解決 gate はこの変更で緩和しない。
接触抜きの予測は server の結果と異なり得る。通常の control API の dispatch は受理ではなく、
`corrections` が実受信補正を数え、補正を自分のモデルへ適用する。
body Move は移動送信中と bounded settle window の補正を `Corrected` として返し、
補正が無い場合は `Accepted`、期限まで送信できない場合は `Unknown` で終わる。

検証用の `scripts/run_contact_best_effort_live.py` は既存の公式 server runner、
original-byte proxy と RCON を使う。native feature で `body_probe` をビルドし、
`--accept-eula --binary target/debug/examples/body_probe --jars <official-jars> --version <version>`
で実行する。近傍 fox/skeleton/horse、kill 後の item、実行中の spawn、living passenger、
移動中の RCON teleport 補正を確認する。
[2026-10-25 の両版の実測](evidence/contact-best-effort-live-20261025.json)には source/binary/JAR の
fingerprint と独立した server 座標を保存した。補正検証は RCON teleport によるものである。

Step 2 の `scripts/run_contact_history_live.py` は両版の opt-in control を使い、
boat と zombie の追跡距離差（native）または server-only marker の乗客（modern）による
rider lifetime 欠落、近傍 living passenger、
移動中の spawn/despawn・scope 再選択、破棄前後の補正を検査する。
上と同じ引数で実行する。公式 server は隔離した loopback world を使い、
proxy は受信 bytes を変更しない。RCON 座標と同じ owner の dispatch を別々に検査する。
履歴警告の [両版の実測](evidence/contact-history-warnings-live-20261025.json) に、
元の typed cause、継続した入力と owner、独立した座標、source / binary / JAR の fingerprint を保存した。

## 未証明地形でのbest-effort歩行（step 5）

両版の`body::Action::Move`と`PlayerControl::start_control_best_effort()`は、
未知chunk/section・未登録state・位置依存shape・衝突データ不足を送信前の拒否にしない。
既知地形では既存の物理予測を使い、地形計算だけが失敗した場合は短い座標probeを送る。
1.21.11でpiston reconstruction全体の履歴が欠ける場合も、警告を残して受信済みの
static block cacheで予測する。そのcellが未知/未対応なら同じbounded probeへ進む。
strictのreconstruction状態やreceiptは書き換えず、遠方の履歴不足を既知の足元の永久停止にしない。
入力値・自分のmode/scale/装備環境の検証はその前に行い、地形fallbackで迂回しない。
エラー文言を比較せず、地形計算の境界で付ける内部の型付き原因を使う。

probeは水平距離最大0.1 block、垂直変位の絶対値最大0.1 block/tick。
斜め入力も水平距離を正規化し、sprintで上限を増やさない。sneakとitem-useは減速する。
床は推定で捏造せず、`on_ground=false`とし、未証明のjump impulseは適用しない。
垂直のmomentumがない場合はその高さで試行し、ある場合は小さく制限して引き継ぐ。
これはバニラ物理の再現保証ではない。地形が利用可能になれば通常の重力・衝突予測へ戻る。
未知cellをair/solidとしてcacheに登録することはない。

`PlayerControl::walking_diagnostics_for(id).terrain`は失敗理由・未証明tick数・証拠待ちを保持する。
新しいown-pose補正または正常な地形計算なしに送れるprobeは最大20 tick（約1秒）。
上限では同じownerが`Paused`になり、座標送信を止めて地形の再試行を続ける。
同じ古いposeを読み直しても、ownerを停止・再作成しても、この予算は補充しない。
新しい補正は座標へ再起点を取り、古いremote数値状態を捨て、同じheld inputで続行する。
単なるvelocity更新や警告の読取りでは予算を補充しない。
bodyは既存のabsolute deadlineで`Unknown`へ終端し、永久に待たない。
`Corrected`は受信したrollback通知であり、失敗した入力の自動再送を意味しない。

有限経路には`PlayerControl::start_path_best_effort(&[SurvivalControl])`を追加した。
1〜120個の入力をconnectionが50ms間隔で所有して送信し、完全送信後は入力を解除する。
厳密な初速0・完全床支持・解放後静止の証明は要求しない。
`PlayerControl::walking_diagnostics_for(id).path`に予定tick数と完全送信数を残す。途中補正でも同じownerで残りだけを実行する。
予定入力の上書きは拒否し、変更には選択ownerの明示停止を使う。
証拠不足等で進めない経路は`(入力数 + 20) × 50ms + 1秒`の期限で停止・解除する。
`Stopped`や完全送信はserver受理そのものではなく、補正の観測は別に行う。

```rust,no_run
use voxrig::client::prelude::*;
# async fn walk(client: &Client) -> Result<()> {
let control = client.player_control(GameMode::Survival);
let path = [SurvivalControl {
    yaw: 0.0,
    input: SurvivalInput { forward: 1, ..Default::default() },
}; 8];
let owner = control.start_path_best_effort(&path).await?;
let record = control.control_record().await?;
# let _ = (owner, record);
# Ok(())
# }
```

`start_control`・`preview_path`・`start_predicted_path`と既存receiptの意味は維持した。
完全予測を要求する診断では従来の拒否を読める。通常送信には上記の明示opt-inかbodyを使う。
connection/world/spawn/owner、mode、死亡・乗車・飛行の照合と入力の有限値・範囲制限を維持する。
inventory等のunresolved gate分割とdig見積りの緩和（step 6–7）は対象外。
公式server実機の再現は`scripts/run_walk_unknown_live.py`を参照。

開始後のPaused／Stopped／期限切れは両版で`control_state_changes_after`へ届く。
[制御状態の変更ログ](control-state-changes.md)を参照。
