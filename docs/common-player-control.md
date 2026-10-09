# モードに応じた共通の基本操作 (#29)

`client.look`・`client.select_hotbar`は現在の受信modeをSDK内で選び、送信直前に再検査する。
[received-mode API](common-basic-actions.md)として、利用側にmode別の分岐を要求しない。

`client.player_control(expected_mode)`は、視線とheld hotbar選択だけを持つ
`PlayerControl`を返す。ハンドルを作る操作はゲームモードを変えず、権限を与えない。
`player_state().game_mode`の受信値を渡し、各dispatch時にadapterの状態境界内で再検査する。
待機中にmodeが変わった場合も`State`で拒否する。modeが未受信なら同様に拒否する。
既存の`survival()`・`creative()`のAPIとmode検査も引き続き使える。

| 受信mode | `look([yaw, pitch])` | `select_hotbar(slot)` |
| --- | --- | --- |
| Survival | 対応 | 0..8に対応 |
| Creative | 対応 | 0..8に対応 |
| Adventure | 対応 | 0..8に対応 |
| Spectator | 対応 | `Unsupported`、packetなし |

Spectatorのheld item選択はvanilla serverが適用しないため、成功したようなreceiptを作らない。
Spectator固有のメニュー・対象へのspectate移動はこのAPIの範囲外。

角度はfiniteなnative degreeで、pitchは-90..90。範囲外のpitch、NaN/Infinity、slot 9以上は
`InvalidInput`となり、packetやlocal rotation/selectionを変更しない。
1.16.1のlookは現在位置を含むpacketを使うため、respawn後などown poseが未取得なら拒否する。
1.21.11のSurvival/Adventure lookは、受信zero velocityかsettled finite motion、通常の立位、
既知の乾いた床・空間等の既存standing検査を使ってground bitを作る。未知のgeometryや
未対応のmotionでは拒否する。Creative/Spectatorのlookにはstandingの推定を追加しない。

新しいハンドルも既存の共通mutation経路を使う。未解決のfinite motion、飛行、乗り物、
収納・inventory操作、pending dispatch、接続の隔離・切断を迂回しない。
modernでheld-slot送信をwriter待機中に取り消した場合、未完了の選択はpendingとして残り、
次のmutationを拒否する。未送信のslotを`Submitted`として公開しない。診断・再接続が必要で、
待機取消や不確実なI/Oから自動再送しない。

戻り値の`DispatchReceipt`は完全なpacket送信だけを表し、server ACK・view/slotの受理ではない。
lookのlocal rotationは`Submitted`となり、以前の`received_pose.rotation`を上書きしない。
選択も`Submitted`と実際のserver選択receiptを区別し、item内容の受信echoを作らない。
lookで送るground bit自体は、接地情報の新しいモデル更新として公開しない。

```rust
use voxrig::client::prelude::*;

async fn aim_in_adventure(client: &Client) -> Result<()> {
    let inputs = client.player_control(GameMode::Adventure);
    inputs.look([45.0, -10.0]).await?;
    inputs.select_hotbar(2).await?;
    Ok(())
}
```

検証手順と実接続の記録は[共通移行APIの検証](common-migration-api-validation.md)を参照。
