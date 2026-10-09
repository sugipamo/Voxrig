# 受信したプレイヤー・ワールド文脈

`Client::player_context()`は両版で同じ`PlayerContextObservation`を返す。
経験値、天候、デフォルトのworld spawn、world-view packetの最後の受信値を、
同じ接続・world generation・capture boundaryで読む。切断後にも読める。
ゲーム内の操作、実行許可、現在時刻への外挿は行わない。

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(client: &Client) -> Result<()> {
    let context = client.player_context().await?;
    if let Some(xp) = context.experience {
        println!("level={}, source={:?}", xp.value.level, xp.source);
    }
    if let Some(rain) = context.weather.raining {
        println!("raining={}, source={:?}", rain.value, rain.source);
    }
    Ok(())
}
```

各fieldの`None`は、そのgenerationで未受信という意味。受信済みのゼロ・falseとは別。
captureの`receive_sequence`が進んでも、変わらないfieldのsourceを更新しない。
world変更、再login、modernのconfiguration変更では全fieldを退役させる。
新しいworldで同じ値が来た場合も、新しい元packetのsourceを保持する。
保存した古いobservationは元のsession・sourceのままで、新しいworldへ再解釈しない。
最新値だけを保持し、dimensional resource-keyの文字列はnativeの32,767byte以内に制限する。

| field | 受信する内容 |
| --- | --- |
| `experience` | 元のprogress float、level、total。相互に計算し直さない |
| `weather.raining` | 元の開始・終了event。強度からbooleanを作らない |
| `weather.rain_level` / `thunder_level` | 元のfloat。render時のblendやthundering booleanを作らない |
| `default_spawn` | worldのデフォルトspawn。個人のベッド・復活先・到達可否とは別 |
| `world_view.center` / `distance` | 元のLOGIN内のdistanceと専用のview packet。loaded chunkやcache budgetを代用しない |
| `world_view.simulation_distance` | modernのLOGINと専用packet。1.16.1は常に未提供 |

1.16.1のspawn packetはblock座標だけ。`dimension`・`yaw`・`pitch`はNoneのまま。
1.21.11は元のglobal positionのdimension文字列・block座標・yaw・pitchを保持する。
そのdimensionが今のplayer worldと同じだとは推測しない。view distanceはLOGIN内で
実際に供給された場合も元のLOGINのsourceを保持する。旧nativeのsuffixを省いた
prefix fixtureでは未提供のままで、値を補完しない。

受信デコーダはfield全体を読んでから更新し、切れた入力・余分な末尾・非有限floatなどを
既存の値へ部分適用しない。負のview distanceは拒否する。
legacy nativeの降雨開始・終了は公式のevent 1＝開始、2＝終了へ修正した。

実接続は`scripts/run_player_context.py`で再生成する。元のpacket ordinalとpayload、
独立RCONの経験値、公式サーバーが保存した`level.dat`の天候、world変更、切断後の読取を
比較する。biome・heightmap・block entity NBT・mapは#41の継続対象。

2026-10-09の公式実接続では両版14項目ずつ、計28項目を確認した。
LOGINのmax players＝5、view distance＝3、modern simulation distance＝2を
別の値にし、受信したview fieldを他のLOGIN fieldと取り違えていないことを照合した。
経験値のゼロ・ポイント追加・level変更、clear／rain／thunder／clear、2地点への
world spawn変更、view center移動、world generation変更、切断後の読取を含む。
新worldの初期XPとコマンド後のXPは、元packetのsourceと独立RCONの全3fieldで
待ち分ける。天候の未受信booleanをclearに補完せず、公式保存データと比較する。
原packet、元SDKビルド・公式JAR・検証スクリプトのhashと境界は
[`evidence/common-player-context-20261009.json`](evidence/common-player-context-20261009.json)
に保持する。
