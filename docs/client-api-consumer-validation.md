# A6: 非公開の利用側で確認する

A0〜A5の代表操作はVoxrig内の同じcommon consumerで両版を通した。
利用側は非公開のまま、`codex/client-api-unification`の提示された固定commitを指定して移行・評価する。
この評価は初回の共通APIを確認する区切りで、[ロードマップのB](client-unification.md)に残る機能を
対応済みとして扱わない。main統合前には、最終候補のcommitでも利用側の結果を確認する。

1. dependencyを提示されたcommitの`voxrig`へ固定し、`Cargo.lock`にも同じcommitが入ったことを確認する。
2. [移行手順](client-api-migration.md)に従い、通常入口を`client::prelude::*`、接続時の版選択を
   `ConnectionConfig.version`または`offline_from_env`に揃える。環境変数は`VOXRIG_MINECRAFT_VERSION`。
   完全な版名`1.16.1`／`1.21.11`を指定し、未対応版はVoxrig更新後に使う。
3. [対応範囲](client-unification.md)と`client.capabilities().support(feature)`を照合し、
   実際の受信modeに合う`survival()`／`creative()`を使う。版固有拡張が必要な操作は記録する。
4. コンパイルと利用プロジェクトの通常の操作を検証する。送信完了・予測・実受信の結果を分け、
   取消／切断後の再送や古いIDの再利用を行わない。全ロードマップの未対応機能を回避しただけの結果を
   全機能移行完了とは扱わない。
5. 使用commit・Minecraft版・mode・操作・結果と不足を返す。ソース公開は不要。

Voxrig側の共通consumerは[common_native_probe.rs](../examples/common_native_probe.rs)。
接続・移動・収納・通常製作・設置、装備／entity、採掘復旧、記録／scene、UI／manager、
かまど、乗車／下車の検証方法と証拠は[実サーバー検証](common-client-native-validation.md)にある。
プロジェクト側でこのfixtureをそのまま運用する必要はなく、そのプロジェクトで必要な操作を確認する。

返却内容の例:

```json
{
  "project": "deepplanning",
  "voxrig_commit": "提示された固定commit",
  "minecraft_version": "1.16.1",
  "mode": "creative",
  "build": "passed / failed",
  "operations": [{"name": "実施した操作", "result": "passed / failed / unsupported"}],
  "remaining_version_specific_apis": [],
  "issues": []
}
```

DustRoute・minetool・deepplanning・golemkitについて、利用中のプロジェクトだけを対象にする。
失敗は操作の前提不足・API差分・Voxrigの不具合・Bの未対応範囲に切り分け、修正対象へ戻す。
