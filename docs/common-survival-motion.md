# 共通Survivalの移動

`Client::survival().preview_path(&controls)`は両版で同じ`MotionPreview`を返す。
版は接続時の`ConnectionConfig.version`に固定され、利用側が版別の操作型を選ぶ必要はない。
現在共通化されているのはread-only previewであり、有限入力列の実行・取消・履歴の共通化は続く。

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
step・tick処理は公式bytecodeにも照合し、実行APIを追加する段階で独立したゲーム結果の確認を行う。
1.21.11の既存native fixture・動作/復旧検査も共有modelへ接続した後に再実行する。

同じ共通API consumerで両adapterのjump/歩行preview、解放後の静止、空/過長入力、
不正yaw/direction、mode違反、position/receiptを変更しないことを検査する。
さらに[共通Clientのnative runner](common-client-native-validation.md)で公式vanilla両版に接続し、
survivalの新しいteleport receipt後に35tickのpreviewを取得する。
RCONで取得前後の実際の位置が`[0.5,65.0,0.5]`のままであることを独立に確認する。
これは移動を実行したという確認ではない。
