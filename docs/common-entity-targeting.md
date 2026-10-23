# 受信targetのentity rayとblock遮蔽

`Survival::target_entities()`と`Creative::target_entities()`は、Java 1.21.11の
受信target boxに対する有界read-only query。共通型`EntityTargetObservation`と
`EntityTargetCandidate`を返す。Java 1.16.1は専用の`Unsupported`を返す。

```rust,no_run
use voxrig::client::prelude::*;
# async fn inspect(client: &Client) -> Result<()> {
let observation = client.survival().target_entities().await?;
for candidate in observation.candidates {
    println!("{:?} point={:?} distance={} block_occluded={}",
        candidate.entity, candidate.point, candidate.distance, candidate.block_occluded);
}
# Ok(())
# }
```

own eye・姿勢・scale・実受信`entity_interaction_range`、entity catalogueとblock geometryを
同じadapter lockで読む。own条件は[姿勢付きblock query](common-survival-targeting.md)と共通で、
healthyな受信mode、静止した実受信位置とzero velocity、Standing/Crouching、scale 0.125〜4、
既知のdry支持とbody clearanceを要求する。未解決dispatch・control・移動・乗車・飛行・死亡・
再構築不足は拒否する。block queryの`block_interaction_range`受信は要求しない。
既存のblock query・操作の条件とerrorは変更しない。

`reach`は実受信属性を原版の0〜64へclampした値とその受信ordinal。
`eye`は原版のpose寸法とfloat scaleから求める`Predicted`で、交点の受信値ではない。
`initial`に元属性と位置・回転の出典、`receive_sequence`に共通capture境界を残す。
各候補の`geometry`は既存の`derived_geometry()`をそのまま保持し、player/cow/pig/zombieの
pose・Baby・scale・位置target・量子化誤差・入力の由来を読める。

候補は最後に受信したpacket targetに置いたboxの交差であり、現在の描画補間位置やserverの
実hitboxではない。候補のpickability・spectator・attack条件はこの幾何queryでは判定しない。
未追跡entityの存在やserverのline of sight、attack成功を証明しない。
queryはpacketを送信せず、位置・在庫を変更しない。保存した結果は操作許可にならない。

## 距離と遮蔽

- 対象4種類の原版`Entity.getPickRadius()`は0。projectile用の0.3 marginを流用しない。
- 交差は原版`AABB.clip`と同じentry-plane計算。X/Y/Z順、平行・面の1e-7 tolerance、
  segment端点のstrict比較を使う。通常の候補は`distance_squared < reach²`。
- 原点がboxのmin以上・max未満なら`eye_inside=true`、pointはeye、距離は0。
  zero reachでもこの包含候補は残る。max面上を内部には読み替えない。
- ownはunmountedに限定し、実受信passenger graphのown rootを共有する外側候補を除外する。
  原版pickerの包含分岐はroot除外より先にあるため、内部候補は`same_vehicle=true`でも残す。
  不明な寿命のedge・矛盾した親・graph上限不足は拒否する。
- `block_hit`は同じeye・entity reachで既存のstatic `OUTLINE/NONE` rayを実行したfirst hit。
  `block_occluded`は候補距離の二乗がblock交点距離の二乗以上の場合にtrue。同距離ではblockを優先する。
  壁の後ろの幾何候補も残し、候補自体の交差とblock遮蔽を別に読める。
- 候補は距離の二乗、同距離では数値IDの診断順。重なる候補は両方残す。
  原版の空間探索順・先に選ばれた距離0の候補に依存する例外・最終一体のpickを再現したとはしない。

block rayは手前の既知hitで止まり、その後の未知領域は先読みしない。
hitより手前の未ロードcell、未知・動く・world依存outlineはquery全体をエラーにする。
entity geometryの未対応種類・欠測scale/pose/Baby/位置も全体をエラーにする。
未解読boxには安全な最大範囲を仮定できないため、遠方の受信targetも黙ってskipしない。
この保守的な制約により、範囲外の未対応entityだけでもqueryを利用できない場合がある。

保持catalogue全体を最大256体まで検査し、超過時は部分結果やtruncateを返さない。
結果も最大256候補、reachは64以下。空結果は、この受信catalogueと利用可能なblock rayの範囲で
資格化済みboxの候補がないことだけを表す。
remove後・同じ数値IDの再spawn後は元の`EntityId`を再利用せず、古い観測はそのまま凍結される。

## 独立した検証

[原版fixtureの出典](../data/client_api/entity_target_rays_source.json)には、公式server JAR・
mappings・generator・fixtureのhashを記録する。[exporter](../scripts/ExportEntityTargetRays.java)は
未変更の原版`AABB.clip`／`contains`を1,521ケースで呼び、対象4種類のpick radiusも照合する。
軸の往復、端点、包含、面のtolerance、対角線、負座標、world境界付近と寸法の変化を含む。
constructorなしのinstanceはfieldを読まないpick radiusだけに使い、worldやtickを作ったとはしない。
原版`ProjectileUtil.getEntityHitResult`のbytecodeで、radius、距離のstrict比較、包含とroot比較の順を照合した。
既存の寸法・view vector・block clip fixtureは別途回帰する。

```sh
# cpは公式bundleの展開済みserver JARとlibraries。Java 21を使用。
java -XX:ActiveProcessorCount=2 -Xmx512M -cp "$VOXRIG_ENTITY_RAY_CLASSPATH" \
  /path/to/Voxrig/scripts/ExportEntityTargetRays.java /tmp/entity-target-rays.json
cargo test --locked --lib -j2 entity_target -- --test-threads=2
python3 -B scripts/run_entity_target_live.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe \
  --jar /path/to/official-1.21.11-server.jar --work /tmp/voxrig-entity-ray
```

Rustの期待値は原版から取得し、Rustの結果をfixtureへ書き戻さない。
constructed packet試験はown/remoteの属性・metadata、壁の更新、remove/ID再利用、未解読geometry、
射程境界・重なり・同一vehicle・未ロード領域・保持上限を検査する。
実接続runnerは隔離serverでplayer/cow/pig/zombieを壁の前後へ置き、元packetとRCON、
queryによる操作packet不在を確認する。実接続の配置確認と、原版clipの数値検証は別の根拠である。

2026-10-23の公式1.21.11実接続は4種類×壁前後の8項目を通過した。
playerは実際の別接続からspawnを受信し、各対象の実受信scaleとown rangeの元packet、
RCONの対象Posと壁のstate、own Pos不変、query操作packet=0を照合した。
[SDK source・binary・元packet/RCONのhashと結果](evidence/entity-target-query-20261023.json)を保持する。
serverとpeerを停止し、一時serverのworld・設定・JARを削除した。

壁より手前の4ケースでも後方の壁のfirst hitを保持し、候補自体は遮蔽されないことを確認した。
