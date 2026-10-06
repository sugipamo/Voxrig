# 共通Clientの採掘後の復旧

両版の`Client::survival().prepare_mining_profile_recovery(mining_id)`は、元の採掘attemptに結び付いた
`MiningProfileRecovery`を返す。準備は通信や切断を行わない。利用側は旧接続を明示的に閉じ、
同じendpoint・版・profileで一度だけ再接続し、返された新しい`Client`で次の計画と操作を行う。

```rust,no_run
use voxrig::client::prelude::*;
# async fn recover(client: &Client, config: ConnectionConfig, id: MiningId) -> Result<()> {
let recovery = client.survival().prepare_mining_profile_recovery(id).await?;
recovery.close_source().await?;
let fresh = recovery.reconnect(config, MiningRecoveryTarget::OriginalOrAir).await?;
// fresh.evidence.originalは閉じた旧attemptの履歴。旧IDで再送しない。
let player = fresh.client.player_state().await?;
let target = fresh.client.survival().target_block(4.5).await?;
// 次の対象・資材・操作を利用側がここから選ぶ。
# let _ = (player, target);
fresh.client.disconnect().await?;
# Ok(())
# }
```

## 対応条件と境界

この方式は、利用側がprofileを専有し、直接接続する未改造の公式vanilla serverに限定する。
Proxy、plugin、別のserver実装の同一UUID処理は同じ契約として監査していない。
実受信したLOGIN_SUCCESSのUUIDと名前を保持し、新しい接続でも照合する。
`Client::connection_identity()`はこの受信事実を返すが、それだけでは復旧を認定しない。

1.16.1のLOGIN_SUCCESSは旧profileの退出より前に送信され得る。
新しいplay JOINと位置受信まで待ち、旧UUIDがPlayerListから除去された後の新しいplayerとして検査する。
旧playerのworld除去がUUID map除去に先行し、閉じたconnectionのtick・queued packetは実行されない。
新playerは新しいServerPlayerGameModeを持つため、元のdelayed miningを持ち越さない。
1.21.11では既存のnative同一profile復旧を使用し、旧profile除去、PLAYER_LOADED送信とfresh admissionを維持する。
元のJARとmappingを照合した[監査要点](evidence/common-profile-recovery-lifecycle-20261006.json)を保持する。
両版とも新しいdimension・位置・健康・全player slot・空cursor・採掘対象・dry standingを確認する。
対象は元のbaselineか通常airに限定され、`Exact`でどちらかを指定することもできる。
読み出したinventoryと対象、standingの受信境界が食い違う場合は組み合わせず、再度取得する。

`MiningRecord.recovery_attempt`は再接続I/Oの前に元の接続へ保持する。
同じwatchのcloneや作り直しからも二度目のloginはできない。失敗・取消でも保持する。
version/profile/endpoint/targetの入力不一致や、元の接続がまだ開いている場合はI/Oとclaimの前に拒否する。
中断・失敗したfresh admissionは新しい接続を閉じる。

旧`MiningRecord.continuation_validated`は常にfalseのまま残る。target air、ABORT、ACK、
切断単独、保存JSONから元接続や旧IDを解放・復元しない。
`MiningRecoveryEvidence`も診断事実で、新しい操作は毎回fresh Clientで検査する。
古い作業の自動再送、途中計画の移植、別UUIDへの切替、工具/液体/未知geometry対応は含まない。

## A3の代表試験

```bash
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py --all --scenario mining-recovery --accept-eula
```

公式serverを順番に起動し、同じconsumerで二つの経路を通す。
通常完了では空手stone採掘、sourceの明示的close、同一profileのfresh admission、
採掘したcellへのstone設置を行う。未解決の場合は早いFINISH後にclose・復旧し、別のcellへ設置する。
元の採掘の推定時間を過ぎても元のstoneが残ることをRCONで確認し、旧delayed miningの継続を検査する。
初期fixture後のRCONは読み取りのみで、採掘後のair、復旧後の設置block、資材3→2、位置不変を確認する。
旧接続の拒否と新接続での旧MiningId拒否、cloneから二度目のlogin拒否もconsumerで検査する。
wire oracleで各経路で旧接続のSTART/FINISH各1回、新接続の設置1回を確認する。
試験結果は[共通native検証](common-client-native-validation.md)へ記録する。


両版の最終成功runは`trial-1.16.1-0d4120cb`と`trial-1.21.11-3d05b30f`。
各版で通常完了・未解決FINISHの二経路が成功し、同じsource/data/binaryとclean JVM exitを確認した。
回帰試験では両版の取消後のshared claim、legacyのLOGIN_SUCCESSだけでは復旧しないこと、
取消されたfresh socketのclose、profile不一致と元接続が開いている間のI/O前拒否も確認した。
