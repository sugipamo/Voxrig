# 共通Clientのコンテナ画面観測

`Client::screen_state()`は1.16.1と1.21.11で同じ`ScreenObservation`を返す。
画面の内容は実受信から保持し、クリック予測やプレイヤー在庫のコピーで補完しない。

```rust,no_run
use voxrig::client::prelude::*;

async fn inspect(client: &Client) -> Result<()> {
    let captured = client.screen_state().await?;
    if let Some(screen) = captured.screen {
        println!("{:?}: {:?}", screen.id, screen.menu_name);
        for (index, value) in screen.slots.iter().enumerate() {
            println!("slot {index}: {value:?}");
        }
    }
    Ok(())
}
```

`active_window == None`は画面が未確定、`Some(0)`は受信で確立されたプレイヤー画面。
`screen == None`だけではclosedを断定しない。OPENを受信せず内容だけ届いた場合は
menuやlayoutを推測しない。閉じた接続へのcaptureはエラーになる。
`player_screen`はlocal UIの操作状態を別に返す。完全送信close由来のSubmittedCloseとreceived player画面を区別し、
actual `active_window`を送信から0へ変更しない。詳細は[共通プレイヤー画面](common-player-screen.md)を参照。

## 画面の識別と受信の由来

`ScreenId`は接続・world generation・numeric window ID・OPEN packetのordinalを保持する。
サーバーが同じnumeric IDを再利用しても新しいOPENは別identityになり、以前のslot/cursorを引き継がない。
serialized historyからlive identityを再構築する入口は提供しない。
login/respawn/reconfigurationで現在のcontainerを破棄する。

`title`はOPEN由来の値。legacyの完全なchat JSONとmodernの完全なunnamed NBTを区別する。
legacy horse window等のtitle欠落は`Unavailable`。表示用textや翻訳結果を捏造しない。
`slots[i] == None`は未受信または完全な対応値が不明、明示的な空slotは`SlotKnowledge::Empty`。
受信slotごとにpacket ordinalがあり、別のslot更新でそのordinalを進めない。
`full_contents_sequence`はそのOPENに対する完全なWINDOW_ITEMSを受信した場合のみ設定する。
partial SET_SLOTだけから全内容が揃ったと推定しない。

legacy WINDOW_ITEMSにはcursorが含まれない。cursorは実際の別SET_SLOTだけで確立する。
modernはfull contentsに実cursorが含まれ、個別cursor packetも保持する。
`revision`はmodernの実state IDとそのpacket ordinal。legacyで相当値を発明しない。
画面の受信revisionはクリック結果の成功証明ではない。

## レイアウト

[ExportMenuLayouts.java](../scripts/ExportMenuLayouts.java)は、未改変の公式server JARから
native menu registryを列挙し、storage menuを実際に構築して各SlotのContainer参照とraw Inventory番号を取得する。
JARとmappingやworldはpackageに含めない。確認済みの結果は
[1.16.1](../data/client_api/menus-1.16.1.json)と[1.21.11](../data/client_api/menus-1.21.11.json)へ保持する。
両版でgeneric 9×1〜9×6、generic 3×3、hopper、shulker boxの9 layoutを確認した。
registry IDは版ごとに異なる（たとえばhopperはlegacy 15、modern 16）。

画面内player slotとcanonical player screen番号の対応は`ScreenLayout::player_slots`に明示する。
たとえばsingle chestはcontainer 0..26、main inventory 27..53 → player 9..35、
hotbar 54..62 → player 36..44。実際の対応値をPlayerObservationにも同じordinalで反映する。
raw Inventory更新も対応する現在のcontainer slotへ反映するが、menu revisionやcursor受信を捏造しない。
未監査のmenuは実slotを保持する一方`layout == None`とし、packet長からplayer offsetを推定しない。
既知layoutとslot件数の不整合、負のindex、layout外の更新は拒否する。

別window宛ての内容・slot・closeで現在のopeningを置換しない。
numeric IDが同じ場合はnativeの順序付きstreamで新しいOPEN以降に受信した値として扱う。
このprotocolには「古いOPEN向けpacket」を別generation付きで識別するfieldは存在しない。

## 実装範囲と検証

`Feature::ContainerObservation`は両版で通常OPEN_WINDOWに対するRestricted。
modernの専用horse openingやその他特殊UIは後続段階であり、通常menuの観測と混同しない。
`Feature::Containers`は既に開いたstorageとhotbarのwhole-stack SWAPに対するRestricted。
両modeの`close_container`も実装し、complete dispatchとactual close replyを区別する。
両modeでempty-handの監査済みstorage activationも実装する。
完全送信とactual OPEN/full/cursorを分離する契約は[共通container open](common-container-open.md)を参照。
通常の左・右クリックとShift転送、製作・装備の対応範囲は[通常クリック](common-inventory-clicks.md)、[Shift転送](common-inventory-transfers.md)、[在庫補助](common-inventory-helpers.md)を参照。
`swap_container_hotbar(screen.id, slot, hotbar)`と`inventory_swap_record()`は両mode・両版に実装する。
取消・receipt・I/O前のcaptureは[共通在庫交換](common-inventory-swaps.md)を参照。
実native fixtureのopenと再openは共通`open_container`を両modeで使用する。
closeの送信では受信screenを消さず、再クリック拒否と履歴は[共通container close](common-container-close.md)で扱う。

legacyは完全なitem NBTを保持する。modernの非default componentも[共通item data](common-item-data.md)の範囲でdecode・保持する。
未対応値をEmptyやdefault itemへ変換しない。unsupported full packetは古いfull-content知識を破棄する。
完全なmodern packetは全field・末尾を検証してから状態へ適用する。

共通consumer fixtureを両adapterで実行し、opening/full receipt、slot provenance、player projection、
numeric ID再利用、別windowへの遅延slot/close、legacy cache予測との分離を検証する。
modernではfull packetの全切断箇所・余分な末尾・件数不一致・範囲外slotが原子的に拒否されることも検査する。
実ゲームの手順と独立RCONの確認は[共通native検証](common-client-native-validation.md)を参照。

## opening付きproperty観測

1.21.11の`Client::screen_properties()`はslot観測とは別の`ScreenPropertyObservation`を返す。
1.16.1は明示`Unsupported`。既存`ScreenObservation`への必須field追加はない。
`screen.id`は`screen_state()`と同じ`ScreenId`だが、別々のcaptureは同一受信境界とは限らない。
値の組み合わせにはsession・opening・各受信ordinalを確認する。

```rust,no_run
use voxrig::client::prelude::*;
async fn properties(client: &Client) -> Result<()> {
    if let Some(screen) = client.screen_properties().await?.screen {
        // menu固有の意味付けはconsumer側でnative menuと照合する。
        if let Some(cook) = screen.values.get(&2) {
            println!("{:?}: cook={:?}", screen.id, cook);
        }
    }
    Ok(())
}
```

CRAFT_PROGRESS_BAR（native `ClientboundContainerSetDataPacket`）のwindowはVarInt、
propertyとvalueはsigned shortのまま保持する。たとえばnative炉では0=燃焼残量、1=燃焼総量、
2=cook進捗、3=cook総量、金床では0=cost、醸造台では0=brew残tick、1=fuel。
このAPIはmenu固有の単位変換・操作許可・完成判定を提供しない。
炉の燃焼や醸造進捗をローカル時計で生成せず、最後の実受信値だけを返す。

`values.get(&property) == None`と受信済みの0を区別し、各値に`ValueSource::Received`の
packet ordinalを保持する。同値再受信でもそのpropertyのordinalを更新し、別propertyは進めない。
未知・負数のproperty/valueもnative値を保持する。最大`MAX_SCREEN_PROPERTIES`（256）件とし、
新しいdistinct keyで超過した場合は最も古い受信ordinalのkeyを退避する。
`evicted`はそのopening中の退避を示し、trueのとき欠測には退避済みも含まれる。
これは最新値の有界captureであり、全変更の履歴ではない。

OPEN未受信のpropertyや別windowのpacketで画面を推測しない。受信close・login/respawn・
reconfigurationで破棄し、同じnumeric IDの再OPENも空から始める。close送信だけでは
`screen_state()`と同様に最後の受信openingを消さず、操作権限も与えない。
その後player window=0の内容受信で画面履歴が残る場合も、元opening宛ての受信closeでproperty観測は失効する。
numeric IDにgeneration fieldはないため、新OPEN以降の同IDへのpacketは新openingへの受信として扱う。
既存の`event_kinds`による画面変更通知だけにはpropertyの値・受信済みkey・opening全体が含まれない。
通知後にgetterを読み、各値のordinalを確認する。通知から全propertyの受信完了を推定しない。

全fieldと末尾を検証してから適用する。packet試験では全切断位置・余分な末尾、同値再受信、
未知値と上限、別window、close、再OPEN、respawnとreconfigurationを検査する。
[ExportScreenProperties.java](../scripts/ExportScreenProperties.java)は公式1.21.11 JARのcodecで
[75件の構築fixture](../data/client_api/screen_properties-1.21.11.json)をencode/decode/re-encodeする。
公式serverとlibrariesをclasspathへ指定し、`javac -cp "$CP" -d "$OUT" scripts/ExportScreenProperties.java`、
`java -cp "$OUT:$CP" voxrig.properties.ExportScreenProperties "$JSON"`で再生成できる。
構築fixtureは実接続の証拠とは区別する。

実接続は[2026-10-23の記録](evidence/common-screen-properties-20261023.json)に保持する。
炉のburn/cookと醸造台のbrew/fuelは同じ受信ordinalの元packetと照合し、tick step完了後の
RCONの実block entity値とも一致した。金床の修理cost=1を受信し、再OPEN時は初回propertyを
proxyで一時保留して新openingの空captureを確認した。金床の材料配置は試験専用のnative
PICKUP入力で準備しており、共通金床操作APIの対応を主張しない。
同じnumeric IDの再利用、malformed packet、保持上限は別の構築packet試験の範囲である。
使い捨て公式serverを使用し、終了時にserverとprobeを停止した。

```bash
cargo build --locked --example climbing_control_probe -j2
python3 -B scripts/run_screen_properties_live.py --accept-eula --server-jar "$MC_JAR" --output "$OUT"
```
