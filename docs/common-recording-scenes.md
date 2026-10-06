# 共通の受信記録・読み取り専用再生・限定scene

1.16.1と1.21.11は同じ`Client`入口を使う。版名以外の利用コードを切り替えない。
保存した記録のdecoderは記録中の完全な版名で選ぶ。未知の版はVoxrig更新が必要。

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(config: ConnectionConfig, region: Region) -> Result<()> {
    let client = Client::connect_recorded(config, 16_777_216).await?;
    client.wait_until_ready().await?;
    let scene = client.survival().capture_scene(region).await?;
    let preview = scene.preview_path(&[SurvivalControl {
        yaw: 0.0, input: SurvivalInput::default(),
    }])?;
    let trace = client.stop_packet_trace().await?;
    let replay = trace.replay(region, 256)?;
    // preview is predicted; replay.received_pose is decoded receive evidence.
    println!("{:?} {:?}", preview.frames, replay.received_pose);
    client.disconnect().await?;
    Ok(())
}
```

## 記録と再生

`connect_recorded`は認証後の最初のconfiguration/play packetより前に記録を有効化する。
`start_packet_trace`は途中区間の診断にも使えるが、その記録には初期JOIN・registry・chunk等の
基準がないため再生を拒否する。開始基準を保存stateで補ったことにはしない。
記録は両版の実受信・適用と同じlock境界を使い、decode前のpayload・元の受信ordinal・phaseを保持する。
相対position correctionには、実decoderが使った直前のlocal position/rotation/velocity入力も保持する。
それらのlocal入力を受信poseとして扱わない。

制限はpayload合計16MiB、最大65536packet。overflow後は`complete=false`を保持して記録を再開しない。
再生は完全な接続開始時点からの記録、連続するordinal、整合するphaseとlocal frame順序、
初期JOIN、queryのdimension境界、chunk数1〜256を要求する。欠けたchunkは`None`で、airにはしない。
JSONへ保存・読み込みできるが、外部入力のJSON自体が本物の受信であることを認証する機能ではない。

再生はsocket・writer・Client・connection actorを作らず、native decoderが生成する応答を送信しない。
返る`ReplayedObservation`は、選んだplayer facts・player inventory・regionのblock値に限定した診断型。
`ReceivedPose`と元のordinal、NBT/component bytes、未知slotとemptyの区別を保持する。
localなhotbar選択は受信slotとして再生成しない。
数値のconnection/window/registry情報は履歴であり、`SessionStamp`・`ScreenId`・`RegistryId`・
操作IDを復元する入口を持たない。`RecordedItemStack`は利用中の`ItemStack`とは別型。

modernは既存configuration/play decoderと受信worldを使う。legacyは既存native parserで
JOIN/RESPAWN、position、health/mode/abilities、選択・在庫・通常画面、chunk/block/unload/explosionを
再生する。選んだ再生面以外のlegacy packet IDは`unhandled_packets`へ明示する。
entityの現在状態、画面の全動作、完全な履歴復元、piston等の広い再構成まで共通化したとは扱わない。

## 限定scene

`Survival::capture_scene(region)`は健康・通常姿勢・stationary・dry・Survivalのnative条件を使い、
loadedなair/passive dry full cubes・登録された乾いたstairs/slabとモデル初期値を一つの境界でコピーする。
stairs/slabの受信propertyと形状の対応は[共通dry terrain](common-dry-terrain.md)を参照。
64cell/axis、32768cell totalが上限で、初期立位の周囲・支持blockもregionに含める。
未観測・未対応形状・不足したhaloは拒否する。

`CapturedSurvivalScene::preview_path`は同じ版のlive previewモデルで1〜120の明示的入力を予測する。
region外の幾何は拒否し、live Clientや保存sceneを変更しない。元のlocal velocityも保存し、
受信resetとresting中のモデルseedを混ぜない。sourceの地形が更新・切断されても元のsceneは変わらない。
`ScenePreview`の保存値、terminal clearance、古いsourceはmutationの実行許可ではない。
実操作はlive側で新しくadmissionする。`CapturedSurvivalScene`はDeserializeできない。

この区切りではcaptureと単独予測を両版へ接続する。仮想block編集、経路の連鎖、assumed scene、
再構成・復旧を含む広いcontext契約はロードマップBに残し、既存modern拡張を維持する。
