# 接続先から受信したregistry

`client.registry()`はVoxrigに同梱した版別のblock-state/item/component型と固定registry entryを検索する。
`client.server_registry_state().await?`は現在の接続で実際に受信したregistryとtagを取得する。
itemと所有情報を同時に取得する場合は`client.received_inventory().await?`を使い、
そのslot/itemの`registry_state()`を参照する。別々のcaptureは再設定境界をまたぐ可能性がある。
両者の数値IDは別の型で保持する。block tagのmemberもblock-state IDへ読み替えない。

1.21.11ではconfigurationのregistry-data packetごとに名前、entry順序、完全なunnamed compound NBT、
受信ordinalを保持する。entryの位置がその接続でのnative IDになる。
known-packsへは空リストを返すため、データ省略をローカルfixtureで補完しない。
omitted/null entry、重複registry/entry、切れたNBT、余分なpacket byteは受信失敗とする。
`FINISH_CONFIGURATION`を受信するまで`bind`/`find`/`resolve`は利用できない。

```rust
use voxrig::client::prelude::*;

async fn inspect(client: &Client) -> Result<()> {
    let received = client.server_registry_state().await?;
    let id = received.find_entry("minecraft:enchantment", "minecraft:unbreaking")?;
    println!("{} = {}", received.entry_name(&id)?, id.value());
    Ok(())
}
```

`ServerRegistryId`はversion・connection ID・configuration generation・registry名を保持する。
別接続や`START_CONFIGURATION`後のsnapshotで古いIDを解決すると失敗する。
respawnだけではregistryを置き換えないため、world generationとconfiguration generationは別に扱う。
保存済みsnapshotは当時のimmutableな観測であり、現在の接続の操作権限ではない。

1.16.1ではjoinに含まれる完全なnamed NBT codecを`legacy_codec()`に受信ordinal付きで保持する。
その`dimension` listから、元の順序で`minecraft:dimension_type`の個別entryを取得する。
各entryの元compound payloadをunnamed root byte付きで保持し、inlineの`name`も省略しない。
取得・名前検査が全て成功してからconfigurationを置き換えるため、失敗時に部分更新しない。
dimensionがないcodecは生データのみ保持し、個別entryは利用不能とする。標準値で補わない。
これはDimensionType全fieldのcodec妥当性やworld参加条件の検証とは区別する。
両版のtag宣言は、版固有のouter formatを解析した`tags()`と元packet全体の`tag_packet()`に保持する。
宣言済みの空tag/registryは空として残し、未受信は`None`とする。
新しいtag packetは宣言全体を置き換え、再設定は旧entry/tagを全て破棄する。
負member ID、重複tag/registry、切れた宣言、trailing bytesは失敗し、以前の観測を部分更新しない。

configuration全体を64 MiBの元registry/tag payload、256 registry、262,144 entryに制限する。
tagは合計65,536件・1,048,576 member、各tagは65,536 memberまでとする。
元bytesに加えて解析済みの名前/member構造を保持するため、64 MiBはプロセスの総メモリ上限ではない。
snapshotはimmutableなentry/tag領域を共有し、観測のたびにNBT全体をコピーしない。

公式vanilla oracleの133 registryはfixtureの事実であり、このAPIに投入しない。
動的registryは実接続がentryを送っていなければ解決できない。
`Registry::builtin_id`/`builtin_id_by_native_id`/`builtin_name`は元の固定registry rootだけを検索する。
`BuiltinRegistryId`は版・registry名・数値IDを保持する。block entryはblock-state IDと別の種類である。
legacyの固定registryは47、modernは95。modernの動的registryは38あり、同期対象は23だけである。
同期されない15 registryにも、fixtureのentryを投入しない。

`find_entry`/`bind_entry`は固定・動的を分類し、共通の`RegistryEntryId`を返す。
legacyのenchantmentは固定、modernのenchantmentは実受信configurationのentryとして扱う。
固定entryは異なる接続でも同じ版の固定rootに属し、動的entryは接続・configurationが異なれば別IDとなる。
`entry_name`は所有範囲を検査する。固定registryへの非標準なserver entry宣言は、固定値と混ぜず拒否する。
任意のitem bytesが指定した接続から来たことを、この明示的ID bindingだけで証明したとは扱わない。
tagやinline holderのnative equals・item全体の意味比較には別途そのfieldと所有範囲の検証が必要である。
inline holderやtag式、一般componentの意味・prototype・NBT/text等価性・hash・容量/slot規則は後続作業。
entry lookupの追加だけでdata付きitem操作を許可することはない。

検証では、共通APIから取得した1.21.11のentry群をnative packetに再構成し、forwardした元payloadの
length/SHA-256・受信ordinalと照合する。両版のtag packetも同様に照合し、legacy codecは元joinのfieldと照合する。
survival/creativeで実受信したenchantment componentのID/levelを、同じ接続の`minecraft:unbreaking`に照合する。
これはそのfixtureの参照解決の検証であり、任意componentの意味解釈やdata付き操作の証拠にはしない。

今回の元JVM実行では1.21.11の23 registry/14 tag registry、1.16.1のjoin codec/4 tag registryを
両modeで照合した。両JVMはexit 0、trace error/未配達は0で、tmpfsのrun directoryもexport後に除去した。
`data/client_api/server_registry_native_evidence.json`に実行時のsource/binary hash、元packet照合、
RCON結果と生ログのdigestを保持する。実行入力は`9651cb2`とstaged変更であり、実行後に追加した
このevidenceファイルや文書を実行入力へ読み替えない。
