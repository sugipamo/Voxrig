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
専用のopen/close、一般クリック、split/shift、製作、装備は後続実装。
`swap_container_hotbar(screen.id, slot, hotbar)`と`inventory_swap_record()`は両mode・両版に実装する。
取消・receipt・I/O前のcaptureは[共通在庫交換](common-inventory-swaps.md)を参照。
ここでのcreative use-on-blockは既存のinteractionを使ったopening fixtureであり、汎用のcontainer open/close契約ではない。

legacyは完全なitem NBTを保持する。modernの非default componentのdecoderは残作業であり、
未対応値をEmptyやdefault itemへ変換しない。unsupported full packetは古いfull-content知識を破棄する。
完全なmodern packetは全field・末尾を検証してから状態へ適用する。

共通consumer fixtureを両adapterで実行し、opening/full receipt、slot provenance、player projection、
numeric ID再利用、別windowへの遅延slot/close、legacy cache予測との分離を検証する。
modernではfull packetの全切断箇所・余分な末尾・件数不一致・範囲外slotが原子的に拒否されることも検査する。
実ゲームの手順と独立RCONの確認は[共通native検証](common-client-native-validation.md)を参照。
