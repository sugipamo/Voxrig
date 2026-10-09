# ゲームモードをまたぐ基本操作

`Client::look([yaw, pitch])` と `Client::select_hotbar(slot)` は、受信済みの現在の
ゲームモードで基本操作を送信する共通 API です。lookは4モードに対応し、
hotbar選択はSurvival / Creative / Adventureに対応します。
プロトコル処理と mode の検査は SDK が持ち、戦闘方針・武器選択・Body の保護は利用側が持ちます。

```rust,no_run
async fn basic(client: &voxrig::Client) -> voxrig::Result<()> {
    let _look_dispatch = client.look([45.0, -10.0]).await?;
    let _slot_dispatch = client.select_hotbar(0).await?;
    Ok(())
}
```

SDK は現在の received mode を読み、adapter の送信境界でその mode が一致することを再検査します。
受信 mode の欠測・途中の mode 変更・未解決操作・停止済みまたは revoke 済みの接続は拒否します。
角度の finite/pitch 範囲と slot 0..8 も検査し、拒否した要求は送信しません。
この API は通常の adapter admission を迂回しません。1.21.11 Survival / Adventure の look は
立位 geometry が必要で、版固有の未対応状態を「基本操作だから」と成功にしません。

返る `DispatchReceipt` はローカルの packet 送信です。回転と選択 slot の変更は `Submitted` であり、
受信 pose や受信 inventory の ACK を作りません。Spectatorのhotbar選択はvanillaが適用しないため、
`Unsupported`としてheld-slot packet送信前に拒否します。look の送信に使う ground bit は
サーバーの接地証拠にはなりません。

`survival().look/select_hotbar` と `creative().look/select_hotbar` は引き続き指定した received mode を
要求します。モードを限定したい利用側は従来の facade を使えます。

任意のexpected modeを保持する`player_control(mode)`も利用できます。
mode別の対応、取消・pending、欠測時の検査は[基本操作の契約](common-player-control.md)を参照。

両 native adapter の TCP 試験で、4 mode の packet、Submitted の由来、受信 pose の保持、
不正値・未解決操作・mode 変更・mode 欠測・revoke 時の送信拒否を確認します。
