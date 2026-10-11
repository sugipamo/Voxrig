# 受信packetと生物モデルの更新順

`LivingPacketModel`は、元packetの適用、補間targetの登録、明示的なモデルtickを
別々の操作として提供する。`LivingEntityTickOrder`は、そのtickで呼ばれるentityの
順序とguardを計算する。どちらも時刻・受信ordinalからnative tickを生成しない。

実受信には`LivingEntityPacketReplay::from_spawn`と`apply_page`を使う。
`EntityMotionObservation.packet`は、stickyな最新position/rotationとは別に、
その受信で実行された元の`EntityMotionPacket`とreceive sequenceを保持する。
位置だけの更新で過去のrotationを再適用しない。spawnは`packet: None`。
同じ履歴pageは再適用しない。モデルfeet、codec base、packet targetは別々に保持する。

| 処理 | Java 1.16.1 | Java 1.21.11 |
| --- | --- | --- |
| 初期relative base | 1/4096のfloor lattice | 元のdouble座標 |
| relativeで未指定のrotation | 現モデルpose | 有効な補間target（なければ現pose） |
| rotation-onlyの位置 | 現モデルpose | 有効な補間target（なければ現pose） |
| entity絶対位置 | 小さいteleportでは現位置を維持 | 距離二乗が4096超、またはtick対象外ならsnap |
| 通常のentity順 | 元の整数hash mapの走査順 | linked tick listのsnapshot順 |
| tick途中の同ID置換 | このlaneでは拒否 | 古いsnapshot寿命を保持、新しい寿命は次tick |
| freeze / chunk guard | 非playerのchunk ticking判定 | frozen時はplayer/player passengerを持つentityを更新 |

legacyはIDゼロ、負ID、probe衝突、容量増減、iteratorによる削除でwrapを跨いだ
entryの遅延走査を保持する。numeric ID順や登録順へ並べ替えない。
modernは同ID置換でも古いsnapshotのbody参照を新bodyへ差し替えない。
passenger callbackは親の直後に元のリスト順で返す。`Ride`は乗客の移動物理の実装を意味しない。

呼び手は各packet適用の`local_authoritative`と`ticking`、tick開始時のmembership、
player class、removed、chunk predicate、passenger関係を明示する。
同じpageはこれらのpacket guardが変わる境界を跨げない。
legacyの`inChunk == false`によるchunk更新や、tick途中の外部membership/predicate変更は
未資格のため拒否する。modernの外部membership変更はsnapshotを保持する。
モデルの別physics stageで得たposeは`set_model_pose`で明示できるが、target/baseは変えない。

実受信からのreplayはcow/pig/zombieのspawn寿命だけを受け付ける。
playerの独自更新、特殊entity、AI、travel、passenger travel、接触ownerへの自動適用、
renderingをこのlaneから実装済みとは扱わない。modernのrelative teleport/correction等は
#40の受信契約に委ね、未資格のoperationを通常のsyncへ読み替えない。
velocity/head yawは独立した受信sampleであり、補間feetを自動で進めない。

履歴のgap、world/connection変更、同IDの新spawn、remove、own-player補正、
選択entityの未資格status/animation、欠落したmetadata/equipment/scaleはreplayを停止する。
失敗pageの部分適用は公開しない。replayはread-onlyな数値モデルなので、呼び手が
connection revoke/closureやowner破棄を検知した時も`invalidate`する必要がある。
`cancel`は補間だけを止め、receiptやcodec baseを捏造しない。tick orderのinvalidateは
途中の走査を破棄し、そのinstanceを再開しない。

保持上限はactive 256体、modern snapshotを含め最大512体、tick当たり256 dispatch、
passenger深さ32。欠落body、cycle、重複関係、超過は拒否する。自動evictionはしない。
履歴は既存の8,192件/8 MiB上限、pageは最大1,024件。replay自体は固定量の状態だけを持つ。

原版比較は`scripts/run_entity_tick_oracle.py`で、未変更の公式client JARを先頭に置き、
元のpacket handlerと`ClientLevel`のentity走査を実行する。自作harnessは登録済みの
原版worldを使い、描画constructorを省略する。cowのtickではAI/travelを隔離し、
元のlegacy `aiStep` / modern `InterpolationHandler`を呼ぶ。実Player subclassでも
guardとcallback順を確認するが、Player travelを資格化する試験ではない。
16スケジュール×12段階×2版の384段階で、packet適用後の位置bit、残step、
remove/同ID置換/passenger/guardを含む更新順を比較する。

```bash
python3 scripts/run_entity_tick_oracle.py \
  --downloads .local/climbing/downloads \
  --client-downloads .local/flight-audit/downloads \
  --work .local/entity-ticks/formal-run
cargo test --locked --lib client::entity::schedule_tests
python3 scripts/run_entity_tick_live.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe \
  --compiled-sdk-revision <binary-source-commit>
```

実接続driverは両版共通APIで、実spawn/motionの元packet・sequenceと履歴を照合し、
一度だけの適用、明示的step、取消し、remove/新spawn/revokeを確認する。
接続中の数値schedule比較も原版fixtureの明示的な境界を使い、native clockとは扱わない。
原版JAR、mapping、class、元packetはignored `.local`に保存し、公開するのは
自作source、数値結果とhash。
公式両版で各13項目、計26項目が成功し、接続中の96段階・2,367 callbackも照合した。
observer/peerの通信traceエラーは0件。固定した実行ファイルのhashを両接続の前後で確認した。
[SDK・成果物・結果のhashと失敗試行](evidence/common-entity-schedule-20261012.json)を参照。

[補間](common-entity-interpolation.md)・[受信motion](common-entity-motion.md)・
[受信履歴](common-entity-history.md)・[近接探索](common-entity-query.md)も参照。
移動ownerへの接触適用は#89、entity別の独自dispatchは#92で扱う。
