# 共通boss bar観測

`Client::boss_bars()`は、接続時に選んだ1.16.1／1.21.11のadapterから
同じ`BossBarsObservation`を返す。mode handleを必要とせず、Survival／Creativeで利用できる。
送信・描画・テキスト参照解決は行わない。

```rust,no_run
use voxrig::client::prelude::*;
async fn inspect(client: &Client) -> Result<()> {
    let observed = client.boss_bars().await?;
    for bar in observed.bars {
        println!("{:?} {:?} {:?}", bar.uuid, bar.title, bar.progress);
    }
    Ok(())
```

## 実受信と部分更新

`ReceivedBossBar`は実際のADDによって成立したbarだけを保持する。
元UUIDはnetwork順の16byteで、操作権限やentity identityではない。
各fieldは個別の`ObservedValue`で、ADDまたはそのfieldを変更したpacketのordinalを持つ。

| packetの操作 | 更新するfield |
| --- | --- |
| ADD | title、progress、color、overlay、flagsを実受信値で設定 |
| REMOVE | 元UUIDのbarを除去 |
| UPDATE_PROGRESS | progressのみ |
| UPDATE_NAME | titleのみ |
| UPDATE_STYLE | colorとoverlayのみ |
| UPDATE_PROPERTIES | flagsのみ |

部分更新で無関係なfieldのordinalを更新しない。同じUUIDのADDは、新しい実宣言として
全fieldを置き換える。ADD未受信のUUIDへの部分更新はbarを作らず、元クライアント同様に無視する。
REMOVEと無視した更新も`last_update_sequence`へ保持する。
最初のpacket未受信ではNone。空の一覧は、サーバーにboss barがないことの保証ではない。

titleは元のlegacy JSON／modern unnamed NBT encodingを保持する。progressは有限の元floatを
保持し、0〜1へclampしたりentityの健康値へ読み替えたりしない。
colorとoverlayは元のenumに対応する共通型。flagsは元byte全体と、
`darken_screen()`／`play_music()`／`create_fog()`の受信指定を読み取れる。
これらの指定から実際の描画・音・霧の状態を予測しない。

## 境界と制約

packet全体を検査してからcacheを更新し、欠損・余分なfield・未知operation／enum・
非有限progressで部分的に採用しない。4096個のbarを上限にし、別Clientのcacheと共有しない。
観測には現在の接続/worldとcapture時のreceive boundaryを保持する。
閉じた接続からのlive観測は失敗する。保存した観測は操作tokenにはならない。
legacyの版固有`UiState.boss_bars`も維持し、共通処理で完全なpacketを検査した後に更新する。

元の未改変packet reader／writer・enumとmodern Handler dispatchで、両版各31例を照合した。
[原codecの値](../data/client_api/boss_bar_packets.json)と
[原JAR・mapping・生成器の固定入力](../data/client_api/boss_bar_source.json)を保存する。
軽量試験は原codecの各操作、部分更新の受信時点、未受信UUID、全truncation／trailing、
不正enum／operation／非有限値による更新拒否、両adapterの実受信と削除・切断を確認する。

これはB6のboss bar観測。teams・titles・tab list・world border等のUI、
特殊window／vehicle／manager、広いB3〜B5と非公開A6は引き続き必要。

両版のSurvival／Creative実ClientでADD→部分更新→REMOVE→manager終了を確認した。
各fieldの元packetとの一致と独立RCONの配信先・value・実削除を別に検証した。
[固定451入力と結果](evidence/common-boss-bars-20261007.json)を参照。
