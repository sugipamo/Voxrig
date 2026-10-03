# 共通Survivalの移動

`Client::survival().preview_path(&controls)`は両版で同じ`MotionPreview`を返す。
版は接続時の`ConnectionConfig.version`に固定され、利用側が版別の操作型を選ぶ必要はない。
`Survival::start_predicted_path(&controls)`は両版で有限入力列を実行し、
`Survival::motion_record()`で共通の`MotionRecord`を参照できる。
独立したobserver契約、さらに広い物理・移動条件と高度な記録/復旧の共通化は後続段階に残る。

```rust,no_run
use voxrig::client::prelude::*;
# async fn preview(client: &Client) -> Result<()> {
let controls: Vec<_> = (0..35)
    .map(|tick| SurvivalControl {
        yaw: 35.57,
        input: SurvivalInput {
            forward: i8::from(tick < 5),
            jump: tick == 0,
            ..Default::default()
        },
    })
    .collect();
let preview = client.survival().preview_path(&controls).await?;
let predicted_endpoint = preview.frames.last();
# let _ = predicted_endpoint;
# Ok(())
# }
```

1〜120個のdigital controls、有限yaw、forward/strafeが-1〜1であることを共通に検査する。
予測は健康な通常立位のsurvival、静止した初期状態、native defaultの移動値、
受信済みのdry full-cube geometryに限定する。歩行とjumpを扱い、sprint/sneak、fluid、
任意のblock形状、動くgeometry、effect付き移動を通常歩行へ補完しない。
不足したchunk、native world boundsを跨ぐ問い合わせ、未対応形状はエラーにする。

`initial`には同じadapter lock境界のplayer/inventoryを保持する。
`initial.position`と`initial.received_pose`は異なる根拠を持ち得る。
`initial_frame`と`frames`はclient modelであり、受信値やserver tickではない。
1.16.1では既存physics cacheが静止を示す場合にそのlocal velocityをmodel seedへ保持する。
1.21.11では既存checked contractのstanding/provenance検査を維持する。
どちらも予測を実際のserver位置や効果の独立確認として使わない。

`terminal_clearance`は、入力が解放されてmodel上で静止し、既知の乾いた支持と水平reserveを
確認できるかを示す。1/16blockのreserveはmodel内の計画値で、実位置誤差の保証ではない。
previewは送信・在庫変更・positionの代入を行わず、再利用可能な操作許可を作らない。

## 有限入力列の実行

`start_predicted_path`は現在の初期状態・地形を再取得し、全入力を予測して、解放後の静止と
既知の乾いた支持を確認してから開始する。以前のpreviewやJSONを操作許可として取り込まない。
返される`MotionRecord`は開始時の診断記録で、更新は`motion_record()`から取得する。
予測契約を選んだことをメソッド名に明示し、受信されていない終点をreceivedと扱わない。

- connection/worldのidentity、同じ境界の初期capture、controlsとforecastをI/O前に保持する。
- 各tickの`attempted_tick`をI/O前、`dispatched_ticks`を完全送信後に記録する。
- 接続側のtaskが50ms間隔で入力を送信する。利用側が読取待機を中断しても再送や中途半端なheld inputを作らない。
- 実行中は別の共通mutation/移動を拒否し、1.16.1の既存autonomous physicsも休止する。
- 1.16.1はactorが通常のprimitive・click・control dispatchを排他する。protocol応答と有限cleanupは維持する。
  capture直前のactor revisionも検査し、排他取得前に別のnative操作が割り込んだ場合は送信を始めない。
- 各tickで再取得した地形の結果が最初の予測と違う場合、位置補正・impulse・mode/姿勢/effect・属性の変更、
  送信失敗や接続終了は`RequiresInspection`へ保持し、入力を自動再開・再送しない。
- `Predicted`は全入力の送信とmodel上の解放後静止を示す。次の操作は現在のcontextと支持を再検査する。
  1.16.1の未解決runは現段階では再接続して調査後に再計画する。追加の観測/復旧契約は後続段階で統合する。

1.16.1ではPosition+Look(0x13)とonGroundを送る。1.21.11ではnative PlayerInputとPosition+Look、
onGround/horizontalCollision flagsを送る。1.16.1へ存在しないdigital-input packetやmodern flag/ACKを追加しない。
両版とも初期の実際のown-pose receiptは別に保持し、送信・予測終点で置き換えない。
1.21.11の既存独立observer、standing admission、reconstructionと履歴ガードも維持する。
native-onlyの別移動へ切り替えた場合は、直前の共通runをsupersededの診断として保持する。

```rust,no_run
# use voxrig::client::prelude::*;
# async fn execute(client: &Client, controls: &[SurvivalControl]) -> Result<()> {
let ops = client.survival();
let started = ops.start_predicted_path(controls).await?;
// 待機・poll間隔・利用側のdeadlineはcallerが選ぶ。次の参照は読取だけ。
if let Some(current) = ops.motion_record().await? {
    assert_eq!(current.run_id, started.run_id);
    let _ = (current.status, current.attempted_tick, current.dispatched_ticks);
}
# Ok(())
# }
```

## 版別規則と検証

共有modelはnative版を明示的に受け取る。1.16.1と1.21.11で違う入力の正規化、
MathHelperのindex丸め、微小velocityの停止、step候補検索、微小変位の適用条件を区別する。
共通型と共通入口を持つことと、版の物理規則が同じであることを混同しない。

1.16.1の[fixture](../data/client_api/java_1_16_1_dry_movement.json)は
SHA-1照合した公式server JARを変更せず、[Java verifier](../scripts/VerifyLegacyDryMovement.java)から
native `Entity.getInputVector`と`Entity.collideBoundingBoxLegacy`を呼び出して生成した。
108件の入力/rotation、72件のvoxel collision、native player dimensions、対象12素材のregistry名、
default collision shapesとfriction/speed/jump multipliersをRust側と比較する。
出所と生成コードのhashは[source record](../data/client_api/java_1_16_1_dry_movement_source.json)に保持する。

再生成は検証済みの公式1.16.1 JARに対して次を実行する。作業用ディレクトリで実行し、
生成JSONをreviewしてからfixtureとsource recordのhashを更新する。

```bash
java -XX:ActiveProcessorCount=1 -Xmx512M -cp /path/to/1.16.1-server.jar \
  /path/to/Voxrig/scripts/VerifyLegacyDryMovement.java /path/to/output.json
```

JDKのcompiler moduleを含むJava 21で検証した。server/worldは起動しない。
このprimitive fixtureだけでintegrated travel、jumpやstepの実ゲーム結果まで証明しない。
step・tick処理は公式bytecodeにも照合した。有限入力列の実行は以下のnative scenarioでも確認する。
1.21.11の既存native fixture・動作/復旧検査も共有modelへ接続した後に再実行する。

同じ共通API consumerで両adapterのjump/歩行preview、解放後の静止、空/過長入力、
不正yaw/direction、mode違反、position/receiptを変更しないことを検査する。
さらに[共通Clientのnative runner](common-client-native-validation.md)で公式vanilla両版に接続し、
survivalの新しいteleport receipt後に35tickのpreviewを取得する。
RCONで取得前後の実際の位置が`[0.5,65.0,0.5]`のままであることを独立に確認する。
これは移動を実行したという確認ではない。

実移動も両版で同じ35tickのjump/歩行コードを実行する。server RCONから途中の位置を複数回取得し、
1block以上のjump上昇、水平移動と予測終点の一致を確認した。これはvanilla serverが位置を受理した
scenarioの結果であり、すべての条件の物理互換や独立observerによる追加の操作許可には読み替えない。
fixtureでは同じconsumerで待機取消後の完了と次の有限run、版別packet形式、競合native操作の拒否、
I/O前のintent、地形変更での停止、送信失敗、零impulseによる無効化、native-only移動へ切り替えた際の診断保持を検査する。
