# voxrig

Minecraft Java Edition 1.16.1（protocol 736、offline-mode）向けのRust製headless clientライブラリです。

外部のAI、planner、behavior treeなどに対する「身体」として、Minecraft protocol、状態同期、クライアント物理、構造化された観測、低レベル操作を提供します。経路探索、意味認識、行動計画、長期記憶といった頭脳は利用側へ委譲します。

```toml
[dependencies]
voxrig = { path = "../mc" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust,no_run
use voxrig::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let manager = BotManager::new(Server::new("127.0.0.1", 25565));
    let bot = manager.connect(Player::offline("AgentOne")).await?;
    bot.wait_until_ready().await?;

    assert_eq!(Bot::protocol_info().protocol_version, 736);

    let blocks = bot.observe_snapshot(4).await?;
    let entities = bot.observe_entities(16.0).await;
    println!(
        "world revision={}, blocks={}, entities={}",
        blocks.revision,
        blocks.value.len(),
        entities.len()
    );

    bot.set_control(ControlState {
        forward: true,
        ..Default::default()
    }).await;
    bot.jump().await?;
    bot.clear_control().await;
    bot.disconnect().await?;
    Ok(())
}
```

対応機能、API、設計上の責任境界、実サーバーでの検証結果は[ドキュメント一覧](docs/index.md)を参照してください。

> [!IMPORTANT]
> 対象はJava Edition 1.16.1のoffline-modeサーバーです。Microsoft認証、online-mode暗号化、他のprotocol versionには対応していません。

## 開発時の確認

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo test --doc
cargo clippy --all-targets -- -D warnings
```

## ライセンス

zen-minecraft-clientは[MIT License](LICENSE)で提供します。組み込まれたregistry dataと
fixtureの出典・ライセンスは[Third-party notices](THIRD_PARTY_NOTICES.md)を参照してください。
