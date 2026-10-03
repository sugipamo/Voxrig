# Voxrig

Minecraft Java Edition 向けのRust製headless clientライブラリです。
既存の1.16.1（protocol 736）実装を維持し、版を選ぶ `Client` API に
1.21.11（protocol 774）の観測・限定的な操作用アダプタを追加しています。どちらもoffline-modeです。

外部のAI、planner、behavior treeなどに対する「身体」として、Minecraft protocol、状態同期、クライアント物理、構造化された観測、低レベル操作を提供します。経路探索、意味認識、行動計画、長期記憶といった頭脳は利用側へ委譲します。

```toml
[dependencies]
voxrig = { path = "../mc" }
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

共通化は専用ブランチ上で段階的に進めています。
`Client::survival()` / `Client::creative()`は両版のモード別入口で、取得してもサーバーのmodeを変更しません。
操作時に受信mode・権限・未解決状態を確認します。対応範囲と残作業は
[Client共通化の実装・検証計画](docs/client-unification.md)を参照してください。
両版の限定的な素手dirt/stone採掘は[共通Survivalの採掘](docs/common-survival-mining.md)を参照してください。
両版のdefault cube設置と材料の受信確認は[共通Survivalの設置](docs/common-survival-placement.md)を参照してください。
新しい版・ブロックへの対応にはVoxrig更新が必要です。`latest`や未知ブロックの推測互換はありません。

公開APIの再設計と各派生版からの移行は[client API設計](docs/public-client-api.md)と
[移行手順](docs/client-api-migration.md)を参照してください。

対応機能、API、設計上の責任境界、実サーバーでの検証結果は[ドキュメント一覧](docs/index.md)を参照してください。

> [!IMPORTANT]
> 従来の `Bot` / `BotManager` とcrate rootの操作型は1.16.1専用です。
> 1.21.11は領域観測・記録・照準・remote player観測に対応し、
> `Client::java_1_21_11_operations()`で限定的なクリエイティブ移動・inventory・設置・除去を扱います。
> `observe_client_region` には、通常・粘着ピストンの移動中状態と階段形状を扱う
> [限定的なクライアント更新機構](docs/client-piston-reconstruction.md)があります。
> 受信状態と計算結果を分けて公開し、不足する処理・情報は明示します。
> 1.21.11では受信した単純スタックの観測に加え、survivalでの
> [メイン所持品・ホットバー間の確認付き交換](docs/survival-inventory.md)に限定して対応します。
> [静止した通常立位の接地判定と自身の受信状態](docs/survival-standing-context.md)も提供します。
> さらに限定的な[通常採掘](docs/survival-mining.md)、[配置](docs/survival-placement.md)、
> [歩行・ジャンプ制御](docs/survival-motion-controls.md)があります。
> `Client::checked_survival()`で検査付きの操作を選び、`survival_capabilities()`で版ごとの対応を確認できます。
> [公開契約](docs/survival-api.md)は経路・権限・永続jobを利用側へ残します。
> 移動後の立位は予測と別接続の観測を区別します。明示的な[予測契約](docs/survival-predicted-motion.md)では
> observerなしでmodel終点を使えますが、実測位置や物理誤差の保証ではありません。
> [同一プロフィールの採掘復旧](docs/survival-single-profile-recovery.md)は直接未改造vanillaの限定契約です。
> 壁に接したまま止まる入力列は送信前に拒否します。
> 元の失敗記録と、退避を含む入力列で配置まで成功した試行記録を保持しています。
> 汎用地形の移動・採掘、汎用container操作、両版のMicrosoft認証・online-mode暗号化は未対応です。
> 対応範囲と失敗した試行は[バージョン別の検証記録](docs/version-adapter-validation.md)を参照してください。

## 開発時の確認

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo test --doc
cargo clippy --all-targets -- -D warnings
```

## ライセンス

Voxrigは[MIT License](LICENSE)で提供します。組み込まれたregistry dataと
fixtureの出典・ライセンスは[Third-party notices](THIRD_PARTY_NOTICES.md)を参照してください。
