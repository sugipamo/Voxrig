# 受信したchunk文脈

`Client::chunk_context([x, z])`は両版で同じ`ChunkContextObservation`を返す。
block stateと光の`Client::chunk()`とは別に、biome、heightmap、block entity NBTの
最後の受信情報を読む。切断後も保存済みの列を読める。unload済みの列は`None`。

```rust,no_run
use voxrig::client::prelude::*;
# async fn inspect(client: &Client) -> Result<()> {
if let Some(context) = client.chunk_context([0, 0]).await? {
    if let Some(biomes) = &context.biomes {
        let native_id = biomes.value.native_id(1, 65, 1);
        println!("biome={native_id:?}, source={:?}", biomes.source);
    }
    for entity in &context.block_entities {
        println!("position={:?}, source={:?}", entity.value.position, entity.source);
    }
}
# Ok(())
# }
```

`ChunkIdentity`は元のconnection/world、列座標、full replacementまたは最初の
partial loadの受信番号を持つ。同じ座標の再読み込みを以前の列として扱わない。
partial updateでは同じincarnationを保ち、更新されないfieldのsourceも変えない。
world変更とreconfigurationは列をすべて退役させる。保存した古いcaptureは元の
identityと受信値を保持する。capture boundaryを各fieldの受信番号へ代用しない。

biomeは4×4×4のquart cellのnative ID。名前へのstatic fallback、playerのbiome
選択noise、気温などの推測は行わない。modernの名前は同じworldに属する受信registry
`minecraft:worldgen/biome`のIDと照合する。legacyにはその受信registryがないため、
名前を補完しない。Yの範囲はlegacy＝0..256、modern＝実際のdimension height。

heightmapはlegacyのnamed NBTとmodernの種類ID/long配列から共通のkindと元long値を
保持する。`first_available_y`は元のcompact valueを読むだけで、今のsurfaceや
安全な足場を保証しない。元array lengthがnative geometryと異なる場合はsampleを
`None`にする。受信したblock stateが変わると新しいcaptureのheightmapを未知に戻し、
block stateからの再計算やnativeのfallback reconstructionは行わない。

block entityはfull chunkと専用更新の両方を受け取る。元の座標、versionごとのkind、
optional NBTを保持する。legacyのupdate actionはregistry type IDと混同しない。
`data=None`は実際のEndTagで、未受信とは別。元のnamed/unnamed bytesは
`encoded_nbt()`で読める。NBTの値・文字列・比較は既存の共通NBT contractに従う。
block変更でそのcellのNBTを失効させる。full chunk以外や失効後のentity listは
`block_entities_complete=false`で、listにないcellを「entityなし」と推測しない。

metadataは読み込まれた列だけで保持し、既存の`max_chunks`と同じ寿命・上限を使う。
1列4096 entity、合計2 MiBのentity NBTを上限とし、各NBTは1 MiB／65536 node／depth 64。
上限を超えたfieldや壊れた更新を部分適用しない。更新対象がunloadedの場合もpacketを
検証するが、列を作り直さない。light-onlyのlegacy列からmetadataを補完しない。

再現用の公式実接続スクリプトは`scripts/run_chunk_context.py`。
`VerifyReceivedHeightmap.java`はoriginal nativeのBitStorage constructor/getを実行する
独立oracleであり、ゲームサーバーは変更していない公式JARを使う。
map pixels/iconsは別の受信データで、このAPIのscopeには入らない。

## 検証

公式1.16.1／1.21.11への同じcommon consumerで、full chunk、専用NBT更新、modern専用
biome更新、block変更による失効、unload/reload、world変更、切断後の読取、保存済みcaptureを
照合する。元packetの全biome ID、heightmapのlong値、block entityの元NBT bytesとfield sourceを
比較し、heightmapはoriginal nativeのstorageで全256cellを独立に読む。
`fillbiome`の端ではnativeのnoiseが隣接範囲を参照するため、サーバーのbiome判定は変更範囲の
内側で行う。受信quart cellにその判定を代入しない。
検証のSDK／official JAR hashと結果は
[保存した証拠](evidence/common-chunk-context-20261009.json)に記録する。
