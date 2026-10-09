# 共通Clientの通常クリックとslot条件

共通`swap_hotbar` / `swap_container_hotbar`は、元のnative constructorで確認したslot条件を
送信前に検査する。shulker boxに別のshulker boxを入れるwhole SWAPは`InvalidInput`で拒否する。
拒否時はpacket・owner・未解決recordを作らず、同じopeningで有効な交換を続けられる。
slot番号や名前の類似から受入れ条件を推測せず、選択した版の検証済みprofileを使う。

1.16.1の最大stack容量もnative値へ揃えた。`warped_fungus_on_a_stick`のupstream登録値64は
nativeでは1だった。元の`data/items.json`とharvest監査のsource hashを保持し、registry loaderで
同じnative ID/nameに対応する実容量を適用する。974種類すべての実item容量とID/nameを照合する。
1.21.11はAIR以外の1,504種類を照合する。AIRは実item stackではなくemptyであり、
empty sentinelの容量値をitem容量に読み替えない。

`Survival::click_inventory` / `Creative::click_inventory`は同じ引数・recordで通常PICKUPを実装する。
`InventorySource::Player`（互換名 `InventoryClickSource`）はcanonical player screen slot 1..45、`Container { screen }`は
元の受信済みopeningのstorageと付属player slotを指定する。native slot番号であり、hotbar indexではない。
元opening、実mode、受信済みsource/cursor、版別slot条件を送信前に検査する。
通常PICKUPの既知itemではlegacy NBTとmodern componentsも保持する。受信と同時に保存したregistryで
意味を解決し、実効item容量を検査する。未知item、容量超過、未解決dataや特殊item overrideは送信前に拒否する。
Leftの空きcursor/空きslotとの移動は、
元JARで照合したdefault constructorのlegacy NBTもそのまま保持する。modern default bundleのLeft移動も
空きcursor/空きslotとの境界に限って認める。bundle内部への収納、Right overrideは後続作業。
1..4はplayer crafting入力、5..8は鎧、9..44は通常在庫、45はオフハンド。結果slot 0は専用のcrafting結果APIを使う。
Shift転送は[共通転送API](common-inventory-transfers.md)で別途実装する。cursor付きcloseは[返却とclose](common-container-close.md)で別に実装する。

```rust,ignore
use voxrig::client::prelude::*;
let intent = client.survival().click_inventory(
    InventoryClickSource::Container { screen }, 0, InventoryClickButton::Right,
).await?;
let latest = client.survival().inventory_click_record().await?;
```

カーソルが空ならLeftは全量、Rightは半分を切り上げて取る。持っている時はLeftが可能な量、
Rightが1個を置く。同一itemはslot/item容量の範囲で結合し、別itemは受入れ可能なwhole stackを交換する。
何も変わらないclickは`InvalidInput`で拒否し、packet・owner・recordを作らない。

`InventoryClickRecord`はI/O前の受信済みsource/cursorと、常に`ValueSource::Predicted`の予測を分ける。
予測は受信在庫へ適用しない。`send.dispatched`はframe全量の送信だけを表し、サーバー結果ではない。
両destinationのfreshな実packet受信と、legacyでは元window/actionの実comparison replyを待つ。
`inventory_click_record()`を読み、`ObservedClicked`を確認してから次の操作へ進む。
途中の受信、送信取消、opening再利用、mode/world/hand変更、復元された競合は自動再送しない。
最初の競合は`RequiresInspection`へ保持する。完了recordは後のmode変更や切断でも履歴として保持する。
legacyの送信はactor所有で、呼出元の取消後も引き受けた一度のwriteを続ける。
modernは取消時の未完了dispatchを保持し、後から似た値を受信しても完了へ読み替えない。

## 元のnative実装へ照合した範囲

公式の未変更server JARと公式mappingを使用する。modernは公式bundleのserverと39 libraryの
全40 classpath entryをSHA-256で照合し、別JAR・変更したmethod bodyを混ぜない。
Java 21のsource-file modeで実行するため、`javac`バイナリを別途要求しない。
実サーバーやworldの起動は行わず、native menu/slot/item/codecメソッドをそのまま呼ぶ。

| 各版の照合 | 件数・内容 |
| --- | --- |
| native item登録 | 975 / 1,505、default容量・empty判定・item override |
| constructor slot条件 | storage 9 menu、player slot 9..44 |
| PICKUP | 18,432、要求count有効11,250、実操作前count有効11,450 / 11,950 |
| SWAP | 7,500、hotbarの両端とslot拒否 |
| packet codec往復 | 120、Empty/非空比較、左右button、window、action/revision |

stone/dirt/egg/saddle/white_shulker_boxで容量64/16/1、空・奇数・上限・上限超過を扱う。
`requested_source` / `requested_cursor`はfixtureへ渡した要求で、`source_before` / `cursor_before`は
元のnative setterが完了した後の実stackを別に読む。要求countの有効性は`valid_requested_counts`、
実操作前countの有効性は`valid_default_counts`に保持する。storage準備時のnativeによる数量制限も
256 / 896ケースとして記録し、要求値を実操作前の値に読み替えない。手動で切り詰めて通常操作へ通さない。
PICKUPの実操作前countが有効な変更10,540 / 10,994ケースは、すべて実cursorも変化する。

shulker box slotは17種類のshulker box itemを拒否する。playerに付属する通常slotはこの制限を持たない。
slotのnative基本容量はlegacy 64、modern 99であり、共通の定数64へ丸めない。
実stackには別のitem容量も適用する。data付きstackはprototype/patchから実効容量を求める。容量を超えるcountを切り詰めて受付しない。
modernの17種類のbundleはitem自身がPICKUPをoverrideするので、通常PICKUPの計算対象へ混ぜない。
SWAPはこのitem overrideを実行しないため、bundleの通常クリックとwhole SWAPは別の条件になる。

legacy PICKUPのnative returnは操作前のclicked slot stackである。一度のクリックで実resyncを
求める比較値は、source非空ならEmpty、source空なら受信済みの非空cursorにする。
nativeはclickを実行してから比較するので、false比較応答をrollbackへ読み替えない。

modernは予測cursor hashが実結果と一致するとcursor更新を省略する。
特に全量を戻してEmptyになる時、Empty hashを送るだけでは実Empty受信の根拠を得られない。
実操作前cursorを比較値にし、実cursor更新を求める。default stackのhash codecはnative holder ID/countと
空patch追加・削除から成り、native HashGeneratorがcomponent hashを要求しないことも確認した。
既存SWAPのEmpty hashも同じ検証済みencoderへ揃えた。default通常PICKUPは実predecessorのhashを同じencoderへ渡す。

data付きmodern PICKUPは、未実装のcomponent/cache hashを捏造せず、受信revisionと異なるrevisionで
完全再同期を要求する。`send.screen_revision`は実受信値、`send.sent_screen_revision`は実送信値、
`send.request_full_resync`はこの要求を表す。この時のEmpty比較markerは実cursorの値・hashではない。
要求の送信だけで成功とはせず、sourceとcursorそれぞれのfresh実受信を必須とする。
サーバーrevisionが途中で変わって再同期されない場合はPendingのまま待ち、自動再送しない。
legacyは元のNBTを保持した比較値と実comparison replyで既存の再同期を使う。

データ付き結果の照合はcountとnative fieldを別々に行う。元NBTのkey順や明示default componentが
受信で正規化されてもtyped意味が同じなら一致する。configuration/tag所有が変わったり、
意味解決ができなかった場合は`RequiresInspection`へ保持する。QUICK_MOVE、cursor付きclose、
特殊item overrideと未解決item dataは後続作業。crafting結果は専用APIの条件に従う。

## exactな鎧・オフハンド操作

通常の `click_inventory(Player, slot, button)` を装備slotにも使う。
装備先を自動選択せず、指定したslotとcursorだけを予測する。
鎧の基本容量1、オフハンドの版別基本容量、itemの実効容量を合わせて検査する。
鎧には元のnative slotが許すitemだけを置き、modernは実効 `equippable` と
その `allowed_entities` の受信済み意味を使う。
取り出しは元のnative enchantment条件に従い、Survivalの束縛された鎧は拒否する。
Creativeの取り出し許可も実modeで判断し、呪いの名前だけからmodernの効果を推測しない。
これらは既存QUICK_MOVEと同じSDKのslot判定を共有する。

受信済みの鎧がslot容量を超えていれば、値を切り詰めず送信前に拒否する。
元のnativeクリックはその不正な前提で通常とは違う数量操作を行う場合があり、
本APIはその状態の一般的な修復を提供しない。

2 slotの占有済み内容の交換は、空cursorから `from -> to -> from` の3回のLeft PICKUPで組み立てられる。
各回の `ObservedClicked` を待つ。これはatomicな交換ではなく、途中取消・競合・不確実な送信があれば
その回の記録と実cursorを調べ、同じクリックを再送しない。具体例の13→45は、
通常在庫のslot13を選び、オフハンド45と交換した後、残るcursorをslot13へ置く。
player screenが変わった場合も別の装備先へ迂回しない。

公式未変更JARのexact equipment PICKUPを各版7,290ケース取得し、item/slot容量内の結果を照合した。
元の上限超過ケースも保持し、SDKは送信前拒否を確認する。
生成器とJAR/出力hashは `data/client_api/equipment_pickup_source.json` に固定した。
既存の648件のmodern実効装備条件と、432件の両版・mode別armor取り出し条件も共通判定で検査する。
さらに公式サーバーへの実接続で、両modeの占有済みオフハンド/鎧交換、装備不適合の送信前拒否、
Survivalの束縛拒否とCreativeの取り外し/返却を、freshなsource/cursorと独立したnative保存物で確認する。
modernの装備は通常 `Inventory` と別の `equipment.head` 等を読む。
[実接続記録](evidence/common-equipment-pickup-20261009.json)に元packetと初回fixture失敗も保存する。
GolemkitのBody移行や、元の保存world不具合の再現はこのSDK検証に含まない。

```sh
python3 -B scripts/export_equipment_pickups.py --downloads "$DOWNLOADS" \
  --runtime-output .local/exact-equipment/oracle --check
cargo build --locked --features native --example climbing_control_probe
python3 -B scripts/run_equipment_pickups.py --accept-eula
```

## 再生成

`DOWNLOADS`には別途取得した`VERSION-server.jar`と`VERSION-server-mappings.txt`を配置する。
`MODERN_CLASSPATH`は公式modern bundleを展開したserver/libraryのclasspathを含むテキストファイル。
公式配布物はpackageへ含めない。

```bash
python3 scripts/export_regular_clicks.py \
  --downloads "$DOWNLOADS" \
  --modern-classpath-file "$MODERN_CLASSPATH" \
  --runtime-output .local/regular-click-oracle \
  --check
```

JVMは1つずつ、heap512 MiB・active processor1で実行する。raw/logは`.local`へ保持する。
確認済みrawを正規化する場合だけ`--normalize-only`を使う。これは新しいnative実行の代わりにはならない。
`regular_click_profiles-*`は判定用の事実、`regular_click_cases-*.json.gz`はnative結果、
`regular_click_packets-*`は元codecのbyte列、`regular_click_source.json`は範囲・入力・生成器・出力hashを保持する。

このprimitive oracleのplayer/worldは未spawnの最小contextであり、通常slotのInventoryと
native default feature flagsだけを供給する。接続、mode、対象screenの所有、送信取消、
他者の変更、復旧、実slot/cursor packetの因果を証明する試験ではない。
共通SWAPには別に両adapter/両modeの送信前拒否と、元openingでの有効な交換のconsumer試験を行う。
通常クリックの接続試験では同じClient consumerを両版へ通し、独立した実server状態とも照合する。

既存Clientシナリオの更新後のnative回帰は両公式版で成功した。
`data/client_api/regular_click_native_evidence.json`に実行時input・raw reportのhash、
storage/player SWAPのfresh receiptsと独立RCON結果を保持する。両JVMは正常終了し、tmpfs runtimeも削除済み。
この回帰は一般PICKUPのowner/transport/receipt APIの完成を意味しない。

通常PICKUPの実接続runは`trial-1.16.1-3275293e` / `trial-1.21.11-a30b17c0`で成功した。
同じconsumerでchest 7を3/cursor 4へsplitし、1個返して4/3、全量返して7/Emptyを受信する。
close後のplayer main/hotbarの取り出し・1個置く・返却・元在庫への復元も行う。
`data/client_api/ordinary_pickup_native_evidence.json`に8完了recordずつ、実行時input・raw hashと
独立RCON結果を保持する。両JVMはexit 0、tmpfs runtimeは削除済み。RCONはslot数量・位置を照合し、
cursor/menu所有は実packet以上の根拠を作らない。過去のfailed native履歴は元のevidenceへ保持する。

modernでは元openingのclose送信が完了している場合、実player screen zeroの全量packetに
含まれるcursorも取り込む。未完了closeや別の新openingへこの許可を持ち越さない。
この全量packetが届くまでは、ローカルcloseだけで実player screenやEmpty cursorを作らない。
