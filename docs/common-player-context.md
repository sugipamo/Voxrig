# 受信したプレイヤー・ワールド文脈

`Client::player_context()`は両版で同じ`PlayerContextObservation`を返す。
能力・難易度、経験値、天候、デフォルトのworld spawn、world-view packetの最後の受信値を、
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
| `abilities` | 元の4bit flagsと飛行・歩行速度。requested flight、game mode、movement属性から補完しない |
| `difficulty` | 元のbyte IDとlock boolean。未知IDも元のbyteとして保持し、peacefulへ補完しない |
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
比較する。biome・heightmap・block entity NBTは[chunk context](common-chunk-context.md)、mapは[地図](common-maps.md)を参照する。

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


## 能力と難易度（2026-10-09）

`abilities`は1個の元packet全体を同じsourceで保持する。`PlayerAbilities`の
`invulnerable()`、`flying()`、`may_fly()`、`instant_build()`はそのflagsを読むだけで、
保存した値から操作権限を復元しない。飛行と歩行の速度は元のfloatであり、属性や装備を
適用した現在の速度ではない。既知の0、false、速度0は欠測と別に保持する。
`difficulty.locked`は元の通知のbooleanで、SDKがserverの設定変更を許可する値ではない。

旧版の能力packetは全9byte、難易度は全2byteを検証してからnative stateとcommon receiptを
更新する。能力packetの切れ・余分な末尾・未知flags・非有限速度では、その直前の能力・
許可・速度を部分更新しない。world変更では共通能力の操作guardに使う元receiptも退役する。
modernで以前捨てていた2つの速度と難易度を共通台帳へ保持する。両版の切断後も読める。

```bash
cargo build --locked --example climbing_control_probe
python3 -B scripts/run_abilities_context.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe \
  --jars /absolute/path/to/downloads --compiled-sdk-revision <build-commit>
```

同じconsumerでsurvival・creative・spectatorの通知、4段階の難易度、無関係packetでのsource保持、
world変更と切断後の読取を検査する。元の9byte／2byteを独立にdecodeし、能力は公式サーバーの
player NBTとも照合する。原packet・NBTはローカルに保存し、公開する証拠は結果とhashに限定する。

nativeとの差の監査では、vitals／属性／effect／time／dimension／現在game modeは既存の
player観測、経験値／天候／spawn／view／能力／難易度はこの文脈観測、biome／heightmap／
block entity NBTはchunk文脈、mapは地図観測へ分類する。旧版のprevious game modeと
item cooldown、追加のLOGIN／RESPAWN条件は共通の受信fieldとしては残件。statistics／
advancements／camera／resource packなどのprotocol面は別のPROTOCOL項目で追跡する。
modernの全packet分岐のparityは、この場面の検証だけでは宣言しない。
