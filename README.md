# Voxrig

Minecraft Java Edition 向けのRust製headless clientライブラリです。
1.16.1（protocol 736）と1.21.11（protocol 774）を、接続時に版を選ぶ共通の `Client` API で扱います。どちらもoffline-modeです。

外部のAI、planner、behavior treeなどに対する「身体」として、Minecraft protocol、状態同期、クライアント物理、構造化された観測、低レベル操作を提供します。経路探索、意味認識、行動計画、長期記憶といった頭脳は利用側へ委譲します。

```toml
[dependencies]
voxrig = { git = "https://github.com/sugipamo/Voxrig", branch = "main" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use voxrig::client::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // VOXRIG_MINECRAFT_VERSION=1.16.1 または 1.21.11
    let config = ConnectionConfig::offline_from_env(Server::default(), "AgentOne")?;
    let client = Client::connect(config).await?;
    client.wait_until_ready().await?;
    let player = client.player_state().await?;
    match player.game_mode {
        Some(GameMode::Creative) => { client.creative().select_hotbar(0).await?; }
        Some(GameMode::Survival) => { client.survival().select_hotbar(0).await?; }
        _ => {}
    }
    client.disconnect().await?;
    Ok(())
}
```

`Client::survival()` / `Client::creative()`は両版のモード別入口で、取得してもサーバーのmodeを変更しません。
player の接地・回転の由来と同じ捕捉境界の扱いは [player capture の契約](docs/player-capture-ground.md) を参照してください。

64ブロックまでの[block raycast](docs/common-blocks.md)と、Adventure/Spectatorを含む
`player_control(mode)`の[基本操作](docs/common-player-control.md)を提供しています。
操作時に受信mode・権限・未解決状態を確認します。対応範囲と残作業は
[Client共通化の実装・検証計画](docs/client-unification.md)を参照してください。
両版の限定的な素手dirt/stone採掘は[共通Survivalの採掘](docs/common-survival-mining.md)を参照してください。
両版のdefault cube設置と材料の受信確認は[共通Survivalの設置](docs/common-survival-placement.md)を参照してください。
通常在庫とhotbarの交換は[共通Clientの在庫交換](docs/common-inventory-swaps.md)を参照してください。
共通Clientの両modeで通常Shift転送を使う場合は[転送契約](docs/common-inventory-transfers.md)を参照してください。
開いたチェスト等の実内容・slot対応は[共通Clientのコンテナ画面観測](docs/common-container-observation.md)で確認できます。
新しい版・ブロックへの対応にはVoxrig更新が必要です。`latest`や未知ブロックの推測互換はありません。

公開APIの再設計と各派生版からの移行は[client API設計](docs/public-client-api.md)と
[移行手順](docs/client-api-migration.md)を参照してください。

対応機能、API、設計上の責任境界、実サーバーでの検証結果は[ドキュメント一覧](docs/index.md)を参照してください。

> [!IMPORTANT]
> 版固有のAPI（`Client::native()`、`voxrig::versions::java_1_16_1` の `Bot` / `BotManager`、
> `voxrig::versions::java_1_21_11`）は既定では無効の feature `native` の後ろにあり、共通APIへの置き換えが済んだものから削除します。
> 移行中は `features = ["native"]` を指定してください。対応表は[版固有API（feature `native`）](docs/native-feature.md)にあります。
> 両版のMicrosoft認証・online-mode暗号化は未対応です。

## 開発時の確認

1.16.1の通信調査では `VOXRIG_TRACE_PROTOCOL=1` により、KeepAliveのフレーム処理・
共有ロック取得・返信write完了/失敗・100ms以上かかった観測処理をstderrへ記録できます。
接続開始時にはプレイヤー名、ローカル/接続先ソケットとSDK世代を記録します。
KeepAlive番号は複数接続で重複するため、接続の対応づけにはこの開始記録を使います。
通常は無効で、1プロセス65,536件までです。write完了はサーバー受信の証明ではありません。

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo test --doc
cargo clippy --all-targets -- -D warnings
```

## ライセンス

Voxrigは[MIT License](LICENSE)で提供します。組み込まれたregistry dataと
fixtureの出典・ライセンスは[Third-party notices](THIRD_PARTY_NOTICES.md)を参照してください。

### Java 1.16.1の期限超過時の接続隔離

`Bot::revoke_connection()` は通常のdisconnectキューを待たずに、当該generationを不可逆に
無効化します。戻り値 `GenerationRevocation` はローカルの新規受付遮断を示します。
進行中の送信・ack・inventoryへの影響は未確認であり、正常logoutやserverの静止を
示しません。owner/readerを停止させ、writerのshutdownを別途開始します。
遮断したBotは再利用せず、保留解除・再接続・再送は利用側が別の証拠で判断します。
