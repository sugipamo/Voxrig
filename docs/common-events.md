# 共通のchange通知（event）

「何が変わったか」を受信連番付きで通知する。中身は観測API（`capture`、`player_state`、
`received_inventory`、`chat_after`、`entity_spawns`等）で読む。1.16.1・1.21.11の両方で使える。
設計の経緯は[設計メモ](event-stream-design.md)を参照。

```rust
let mut cursor = 0;
loop {
    let log = client.wait_for_events(cursor, Duration::from_secs(30)).await?;
    for event in &log.events {
        match event.kind {
            EventKind::BlocksChanged { min, max } => { /* capture(Region { min, max }) */ }
            EventKind::Disconnected => return Ok(()),
            _ => {}
        }
    }
    cursor = log.cursor;
}
```

## 読み出し

- `events_after(cursor)`: `cursor`より後のeventを古い順に返す。最初は`0`、以後は返り値の`cursor`を渡す。
  `cursor`はeventの通し番号で、受信連番ではない（1パケットから複数のeventが出るため）。
- `wait_for_events(cursor, limit)`: 1件以上届くまで待つ。上限を過ぎると`ErrorKind::Timeout`。
- 各eventの`receive_sequence`は原因になったパケットの受信連番で、観測APIの受信連番と同じ軸。
  返り値の`receive_sequence`は、そのlogがどの受信連番まで完全かを示す。
- 接続ごとに最新4096件を保持する。保持範囲より古い`cursor`は`ErrorKind::State`で失敗する。
  その場合は観測APIで現在の状態を取り直してから、新しい`cursor`で続ける。
- 切断後も読める。最後のeventは`Disconnected`で、その後は何も記録されない。
  `wait_for_events`も未読の最後のeventを先に返す。その返り値の`cursor`から再び待つと受信エラーを返す。
- `EventKind`は`#[non_exhaustive]`。種類は今後増える。

## 種類と元のパケット

| `EventKind` | 1.16.1（既存`Event`からの対応） | 1.21.11（パケット） |
| --- | --- | --- |
| `BlocksChanged { min, max }` | `BlockChanged`、`DiggingAcknowledged`、`BlockEntityUpdated`、multi-block変更、爆発の非空消失座標一覧（変更位置の範囲） | `BLOCK_CHANGE`、`TILE_ENTITY_DATA`、`MULTI_BLOCK_CHANGE`（section全体の範囲） |
| `ChunkLoaded` / `ChunkUnloaded` | `ChunkLoaded` / `ChunkUnloaded` | `MAP_CHUNK` / `UNLOAD_CHUNK` |
| `InventoryChanged` | `InventoryUpdated`、`SlotUpdated`、`HeldItemChanged` | window 0の`WINDOW_ITEMS`/`SET_SLOT`、`SET_PLAYER_INVENTORY`、`SET_CURSOR_ITEM`、`HELD_ITEM_SLOT` |
| `ScreenChanged` | `WindowOpened`、`WindowClosed`、`WindowProperty`、`MerchantOffers` | それ以外のwindowの`WINDOW_ITEMS`/`SET_SLOT`、`OPEN_WINDOW`、`CLOSE_WINDOW`、`CRAFT_PROGRESS_BAR` |
| `PlayerChanged` | `Position`、`PositionCorrection`、`Vitals`、`Experience`、`GameStateChange`、`SurvivalStateUpdated` | `POSITION`、`UPDATE_HEALTH`、`EXPERIENCE`、`ABILITIES`、`GAME_STATE_CHANGE` |
| `WorldChanged` | `Login`、`Spawn`、`Respawn` | `LOGIN`、`RESPAWN`、`START_CONFIGURATION` |
| `EntitySpawned { native_id }` / `EntityRemoved { native_id }` | `EntitySpawned` / `EntitiesDestroyed` | `SPAWN_ENTITY` / `ENTITY_DESTROY` |
| `ChatReceived` | `Chat` | `SYSTEM_CHAT`、`PLAYER_CHAT`、`PROFILELESS_CHAT` |
| `UiChanged` | `UiStateUpdated`、`PlayerListUpdated` | scoreboard・boss bar・teams・player info・title・tab list・world borderの各パケット |
| `ResourcePackRequested { uuid }` | この共通通知は未対応 | configuration・playの`ADD_RESOURCE_PACK`。最新の要求は[`resource_pack_request`](common-resource-packs.md)で読む |
| `CodeOfConductReceived` | このpacketは存在しない | configurationの`CODE_OF_CONDUCT`。最新の本文は[`code_of_conduct`](common-code-of-conduct.md)で読む。自動acceptの受理通知ではない |
| `LegacyDiggingAcknowledged { position, block_state_id, status, successful }` | 掘削ACKの元の受信結果。拒否・遅延・未対応ACKも保存。[訂正の扱い](digging-ack-corrections.md) | 未発行（processing ACKには状態・成否がない） |
| `ExplosionReceived` | `Explosion`。消失座標・運動量は[`explosions_after`](common-explosions.md) | `EXPLOSION`。payloadは同API |
| `SoundReceived` / `SoundStopped` | この共通通知は未対応 | `SOUND` / `SOUND_ENTITY` / `STOP_SOUND` |
| `ParticlesReceived` | この共通通知は未対応 | `LEVEL_PARTICLES` |
| `LevelEventReceived` | この共通通知は未対応 | `LEVEL_EVENT`（world event） |
| `BlockDestructionReceived` | この共通通知は未対応 | `BLOCK_DESTRUCTION`（他のbreakerのcrack進捗） |
| `ItemPickupReceived` | この共通通知は未対応 | `TAKE_ITEM_ENTITY`（pickup animation） |
| `ObservationInvalidated` | チャンク／光の受信失敗時に直接記録。接続全体の観測を撤回し、reader を終了。[回復と診断](world-decode-failures.md) | 未発行（既存の受信失敗動作を維持） |
| `Disconnected` | `Disconnected`、接続エラー | 切断を検知した最初の読み出し時 |
| `EntityUpdated { native_id }` | `EntityUpdated`（移動・向き・速度・metadata・装備） | `SYNC_ENTITY_POSITION`、`REL_ENTITY_MOVE`、`ENTITY_MOVE_LOOK`、`ENTITY_LOOK`、`ENTITY_HEAD_ROTATION`、`ENTITY_METADATA`、`ENTITY_VELOCITY`、`ENTITY_EQUIPMENT`、`ENTITY_TELEPORT` |
| `EntityStatus { native_id, status }` | `EntityStatus` | `ENTITY_STATUS`（codeは版ごとに違う） |
| `EntityDamaged { native_id }` | `EntityStatus`のうち被害のcode（2・33・36・37・44） | `DAMAGE_EVENT` |
| `HurtAnimationReceived { native_id }` | この共通通知は未対応 | `HURT_ANIMATION` |
| `CombatChanged` | この共通通知は未対応 | `ENTER_COMBAT_EVENT` / `END_COMBAT_EVENT` |
| `PlayerKilled { native_id }` | 自分の死亡（combat eventのdeath） | `DEATH_COMBAT_EVENT` |

- `BlocksChanged`の範囲は実際の変更を含む上位集合。1.21.11のmulti-block変更はsection（16×16×16）単位になる。
- entityのeventは`EntityId`ではなく`native_id`を持つ。操作に使う`EntityId`は`entity_spawns()`で対応する
  ものを引く（`EntityId`はworld単位の識別を含み、event記録時点では安全に作れないため）。
- 各eventの`received_after`は、接続のevent logを作ってからそのeventを記録する（パケットを適用する）までの
  時間（clientの時計）。移動の標本の到着時刻などに使う。
- 1.16.1の`Bot::subscribe()` / `Event`はネイティブAPIとしてそのまま残る。

## 1.21.11の一過性packetの構造化観測（#46）

sound・particle・level event・break progress・pickupは現在値から復元できないため、
`Client::effects_after(after_sequence)`でoriginal payloadの構造化履歴を読む。
イベントは上表の通知だけを持ち、既存の`EventKind` / `ClientEvent`の`Copy`・`Eq`を維持する。

```rust,no_run
use voxrig::client::{effects::PlayEffect, prelude::*};
async fn read(client: &Client) -> Result<()> {
    let mut after_sequence = 0;
    let log = client.effects_after(after_sequence).await?;
    for received in log.effects {
        if let PlayEffect::ItemPickup { collected_id, collector_id, amount } = received.effect {
            println!("{collected_id} -> {collector_id}: {amount}");
        }
    }
    after_sequence = log.receive_sequence;
    let _next = client.effects_after(after_sequence).await?;
    Ok(())
}
```

- `EffectLog::receive_sequence`を次のcursorとして渡す。これは受信連番であり、
  `EventLog::cursor`（通知の通し番号）とは別。初回は0。通知とpayloadは同じ受信連番を持つ。
- `ReceivedEffect::session`は受信時のconnection・version・world。respawnや再設定の後も
  古い標本はそのworld付きで残る。native entity IDを現在のworldの操作対象へ流用しない。
- 最新1024件（`MAX_RETAINED_EFFECTS`）、1 packetは展開後16 KiB以下
  （`MAX_EFFECT_PACKET_BYTES`）。particleの`count`個のobjectを生成しない。
  既存の4096件の通知履歴とは独立して保持する。保持できるpacket bytesの合計は最大16 MiBで、
  構造体・文字列・componentの管理領域は別に必要となる。
- 落とした標本より古いcursorは`ErrorKind::State`。一過性payloadは再観測で復元できない。
  完全な履歴として扱うのをやめ、現在の受信連番（例: `received_player_snapshot()`の`boundary().ordinal()`）を
  明示的に取得して再開する。受信より先のcursorは`InvalidInput`。切断後も読める。
- 1.16.1の共通`effects_after`は`Unsupported`。既存のnative event APIはそのまま使える。
- 切り詰め・既知layoutの末尾余剰・不正なbool/source・非有限float・上限超過は
  受信のprotocol errorとなり、そのpacketのpayloadも通知も記録しない。

| `PlayEffect` | 保持するoriginal値 |
| --- | --- |
| `Sound` | sound holder（zero-based registry ID、またはinlineの名前とoptional固定距離）、source ordinal、1/8 block単位の整数座標またはentity ID、volume・pitch・signed seed |
| `SoundStopped` | optional sourceと名前。両方Noneなら全soundの停止要求 |
| `Particles` | limiter/always-show、座標・offset・speed・signed count、native type ID、型別options |
| `LevelEvent` | 未知の値を含むevent code、block座標、signed data、global flag |
| `BlockDestruction` | breaker ID、block座標、unsigned progress byte。0..=9はstage、それ以外はcrackの解除 |
| `ItemPickup` | collected entity ID、collector entity ID、signed amount |

particleの115種類はRustのconst tableでlayoutを選ぶ。block state・packed color・dustと色遷移・
power/spell・sculk roll・shriek delay・item component patch・vibrationのblock/entity destination・
trailを型付きで保持する。未知のparticle IDやvibration sourceは、推測せず全option bytesを
`ParticleData::Unknown`に保持する。sound registry IDから名前を静的に推測することもしない。

これらはサーバーから受け取った提示用の事実。soundの再生・可聴性、particleの描画、
blockの実際の除去、inventoryの増加やentityの消滅は保証しない。
`TAKE_ITEM_ENTITY`は他のcollectorやexperience orbを含み得る。break progressのbreakerも
現在追跡中のplayerとは限らない。これらだけで`InventoryChanged`や`BlocksChanged`を捏造しない。
Microsoft認証・署名はこの対応のスコープ外。

公式1.21.11 server JARの変更していないcodecで構築packetをdecodeし、同じbytesへencodeし直した
135例を[`effect_packets-1.21.11.json`](../data/client_api/effect_packets-1.21.11.json)に保存した。
registryのID・名前・option classも実物から取得している。
`Server`のoriginal bundled JARとlibrariesをclasspathへ入れ、使い捨てdirectoryで次を実行すると再生成できる
（出力先を明示し、JVMのlogも使い捨てdirectoryへ置く）。

```bash
java -Xmx512M -XX:ActiveProcessorCount=1 -cp "$ORIGINAL_SERVER_CLASSPATH" \
  /absolute/path/to/Voxrig/scripts/ExportEffectPackets.java /tmp/effect-packets.json
```

`examples/effect_events_probe.rs`は`CMD`行をconsoleへ転送するdriver用。
`python3 -B scripts/run_effect_events.py --accept-eula --server-jar /path/to/server.jar
--binary target/debug/examples/effect_events_probe --output /tmp/effect-live`で実行する
（先に`cargo build --locked --example effect_events_probe -j2`）。
使い捨てoffline serverに対し、sound・停止・particle・pickup・もう一人の通常採掘によるbreak progress・
level eventを検証する。driverが空きport（25572・25581を除外）を選び、original traceを
`--output`配下の`live-trace.json`へ保存する。probeを別のdriverから実行する場合は、
`VOXRIG_PORT`でportを、`VOXRIG_EFFECT_TRACE`でoriginal traceの保存先を指定する。

2026-10-22、公式1.21.11 JAR（SHA-1 `64bb6d763bed0a9f1d632ec347938594144943ed`）の
使い捨てserverで上記6種類の通知とpayloadの同一受信連番を確認した。
通常の採掘でbreaker ID 3・`[2,65,0]`のstage 0を受け取り、pickupはcollector ID 1・3個、
続くworld eventは2001・data 1だった。切断後の履歴も読めた。serverは終了・削除済み。
[`検証記録`](evidence/effect-events-20261022.json)にoriginal packet bytes・結果・SDK file hashesを保存した。
`SOUND_ENTITY`は公式codec fixtureで検証し、このlive scenarioでは個別に発生させていない。

## Java 1.21.11 の damage・hurt・combat 詳細（#46）

`Client::effects_after(receive_sequence)`はdamage・hurt・combatも保持する。
変更通知の`EntityDamaged`／`HurtAnimationReceived`／`CombatChanged`／`PlayerKilled`と
payloadの`receive_sequence`を照合する。通知cursorと受信連番cursorは別である。

| `PlayEffect` | 元packetから保持する値 | 変更通知 |
| --- | --- | --- |
| `Damage(DamageObservation)` | entity ID、damage typeの接続・configuration付きIDと受信名、optional cause/direct entity ID、optional source位置 | `EntityDamaged` |
| `HurtAnimation` | entity ID、degree単位のyaw（正規化しない） | `HurtAnimationReceived` |
| `CombatEntered` | payloadなし。自分のcombat trackerの通知 | `CombatChanged` |
| `CombatEnded` | signed duration（server tick）。attacker IDはこの版では送られない | `CombatChanged` |
| `CombatKill` | player ID、完全なunnamed NBTの`UiText` | `PlayerKilled` |

`DamageObservation::source_type`はzero-basedの`ServerRegistryId`で、
`source_type_name`はそのpacket受信時の`minecraft:damage_type` entry名。
実際に受信したregistryの順序を使い、vanillaの固定ID一覧で補わない。
registry未受信・未完了・範囲外IDはprotocol errorとなり、payloadも通知も記録しない。
保存済みの名前と所有情報は再設定後も当時のまま残る。現在のregistry snapshotで旧IDを
再解決すると所有検査で失敗する（[server registry](common-server-registries.md)）。

cause/directはwireの0を`None`、それ以外をJavaのsigned int演算で1引いたIDとして保持する。
entityが追跡範囲外でも破棄せず、現在のentity位置へ置き換えない。
source位置がない場合もentity位置から補完しない。hurt yawはdamage sourceの方角推定とは別の受信値。
damage量・最終health・攻撃の成功はこの通知だけでは分からない。
killは履歴内に元メッセージを保持し、自分のIDなら既存の`death_message()`も同じ受信連番で更新する。

既存effectsと共通で1024件・1packet 16 KiBの保持上限、gapエラー、切断後の読み出しを使う。
解決済みdamage type名は別に保持し、受信identifierの最大32,767 ASCII bytesまでを各標本で使用し得る。
切り詰め・末尾余剰・不正bool・非有限yaw/座標・切れたNBTはprotocol errorで部分記録しない。
1.16.1の`effects_after`は引き続き`Unsupported`で、既存のdamage/death通知とnative combat APIを使う。
Microsoft認証・署名はこの対応のスコープ外。

再現用consumerは`examples/damage_details_probe.rs`。公式JARの使い捨てserverで、
次のdriverが`CMD`行をconsoleへ転送する。空きportを選び、25572・25581を除外し、終了時にserverを停止・削除する。

```bash
cargo build --locked --example damage_details_probe -j2
python3 -B scripts/run_effect_events.py --accept-eula \
  --server-jar /absolute/path/to/server.jar \
  --binary target/debug/examples/damage_details_probe --output /tmp/damage-live
```

2026-10-22に上記consumerで全項目を確認した。受信registryではgenericがID 18、
mob_attackがID 28で、原因entity 3・直接entity 2、hurt yaw 0、combat end 314 ticksだった。
kill messageと既存`death_message()`のbytes・受信連番が一致し、切断後も履歴を読めた。
元packet・受信registry・SDK/JAR/binary hashは[検証記録](evidence/damage-details-20261022.json)に保存した。
全7packetをunit testで再適用し、6件のlive観測と照合する。serverはexit 0で終了・削除済み。

## 死亡の文言・切断の理由・接続状態

- `Client::death_message()`: 自分の最後の死亡の文言（1.16.1はnativeのJSON、1.21.11はNBTの`UiText`）と受信連番。
  次の死亡まで残る。切断後も読める。
- `Client::disconnect_reason()`: serverがkickで送った文言。切断後も読める。
- `Client::connection_status()`: `Joining`（login・設定・最初の同期）、`Ready`、`Closing`、`Closed`、`Unknown`
  （送受信の結果が分からない終わり方）。
- 1.16.1は待機中も毎tick位置を送る。serverがkickの直後に接続を閉じると、この送信が先に失敗して状態は`Unknown`になる。
  そのときも、すでに届いているframeを最大200 ms読み、kickの文言だけを記録する（ほかのパケットは適用しない）。
  文言は状態が変わった少し後に入る。読む前に送信の失敗で受信bufferが破棄された場合は残らない。
- 1.21.11は待機中に送らないので、kickは`Closed`と文言になる。
- 2026-10-08に両版の公式serverで確認した: 豚のteleportで`EntityUpdated`、即時ダメージの効果で
  `EntityDamaged`、`kill`で自分の`PlayerKilled`と死亡の文言、`kick`で文言（1.16.1は`Unknown`、1.21.11は`Closed`）を
  受け取った（`examples/entity_events_probe.rs`）。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`をoffline-modeでlocalhostに起動し、`examples/event_probe.rs`で確認した。
consoleから`setblock`・`fill`・`summon`・`kill`・`tellraw`・`title`・`give`を送った。

| 版 | 届いた種類 | `setblock`位置を含む`BlocksChanged` | 受信連番の順序 | 切断後 |
| --- | --- | --- | --- | --- |
| 1.16.1 | BlocksChanged、ChunkLoaded、EntitySpawned/Removed、ChatReceived、InventoryChanged、PlayerChanged、UiChanged | `[214,7,-227]..[218,8,-225]` | 昇順 | `Disconnected` |
| 1.21.11 | BlocksChanged、ChunkLoaded、EntitySpawned/Removed、ChatReceived、InventoryChanged、UiChanged | `[-16,-64,0]..[-1,-49,15]`（section） | 昇順 | `Disconnected` |

同じblockを置き直すとサーバーは変更を送らないため、確認ではいったんairにしてから置いた。


## nativeイベント源の容量を維持する（#19）

1.16.1の既存`Bot`から共通`Client`へ移すとき、接続前に次を指定できる。
`Client::connect`、`connect_recorded`、`ClientManager::connect`は同じ接続設定を使う。

```rust,no_run
use voxrig::client::prelude::*;
use voxrig::versions::java_1_16_1::Event as NativeEvent;
use tokio::sync::broadcast::Receiver;

async fn connect() -> Result<(Client, Receiver<NativeEvent>)> {
    let mut config = ConnectionConfig::offline(
        Server::default(), "AgentOne", MinecraftVersion::Java1_16_1,
    );
    config.limits.max_chunks = 256;
    config.limits.native_event_channel_capacity = Some(8192);
    let client = Client::connect(config).await?;
    // features = ["native"] の場合、同じtransportの元イベント源を購読できる。
    let events = client.java_1_16_1()?.subscribe();
    Ok((client, events))
}
```

この設定は接続時にnative `ConnectionOptions.event_channel_capacity`へ渡る。
下流relayではなく、`EntitySpawned`／`EntityUpdated`の位置・速度payloadを持つ元のbroadcast源に適用する。
別のBotを接続したり、接続済みBotのsession・receipt・revocationの所有権を移したりしない。
`None`は既存のnative既定値256を維持する。接続後の容量変更は行わない。
0と`usize::MAX / 2`を超える値は、通常・記録付き接続ともnetwork I/O前に`InvalidInput`となる。
Tokioは要求容量を次の2の累乗へ切り上げる場合があり、たとえば1000は1024件となる。
大きな容量は接続時のメモリ使用量を増やす。

現在native broadcast源を持つのは1.16.1だけで、1.21.11に`Some(...)`を指定すると
接続前に`Unsupported`となる。1.21.11の既定`None`は今までどおり接続できる。
設定自体は共通APIで利用でき、元のnativeイベントの購読にはfeature `native`が必要。

共通の`events_after`／`wait_for_events`は引き続き4096件の変更通知であり、native payload履歴にはならない。
nativeの容量内に収まる間は、共通履歴があふれても元のpayloadを取得できる。
容量を超えたnative receiverはこれまでどおり`Lagged(n)`で欠落を通知する。
既存のrelayと、その欠落時に運動推定を破棄する処理はそのまま利用できる。

検証では実TCPで8192件のfireball spawn・位置・速度更新を送り、既定256の`Lagged`と、
指定8192の完全なpayload履歴を確認する。通常・記録付き接続を検査し、共通履歴のoverflowも独立に確認する。
実サーバーの再検証は以下（公式JAR、Java 21、EULA同意が必要）。

```bash
cargo build --locked --example native_event_capacity_probe --features native
python3 scripts/run_native_event_capacity.py --accept-eula \
  --binary target/debug/examples/native_event_capacity_probe \
  --jars /absolute/path/to/downloads
```

1.16.1では4096件を超える元entity更新を保持し、1.21.11では既定設定での共通接続を確認する。
結果は`.local/climbing/live/*/report.json`に保存する。

実接続では1.16.1で6,806件のnativeイベント（entity payload 6,726件）を欠落なく保持し、
共通履歴のoverflowと過去の位置payloadの違いを確認した。1.21.11の既定接続も通過した。
[検証記録](evidence/native-event-capacity-20261008.json)に結果と元reportのhashを保存した。

## 1.16.1 の掘削ACKによる訂正

成功・拒否のいずれもACKのblock stateをworldへ適用し、同じ受信連番で
`BlocksChanged`、`LegacyDiggingAcknowledged`の順に記録する。同じ値のACKも省略しない。
前者は依存地形・経路の無効化、後者は元の成否とactionの履歴に使う。
`successful=true`は除去成功の証明ではない。現在値は再観測する。
詳細は[掘削ACKの訂正と結果](digging-ack-corrections.md)を参照。
