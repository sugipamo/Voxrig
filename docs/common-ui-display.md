# 共通title・tab見出し・world border観測

`Client::titles()`、`Client::tab_list()`、`Client::world_border()`は、setupで選んだ
1.16.1／1.21.11の受信を同じ型で返す。Survival／Creativeの両方で利用でき、送信は行わない。
`Feature::DisplayObservation`で対応範囲を確認する。

## Titleとaction bar

`TitlesObservation`はtitle・subtitle・action bar・timing・最後のclear命令を保持する。
各fieldの`ObservedValue::source`はその値を運んだ実packetのordinalであり、capture全体の
`receive_sequence`とは別である。textは元のlegacy JSON／modern unnamed NBTを保持する`UiText`。

`title`と`subtitle`は二段階のOptionを持つ。外側の`None`は未受信、
`Some(ObservedValue { value: None, .. })`は実CLEAR／RESETによる消去である。
`clear.value=false`はCLEAR、`true`は時間もresetする命令で、最後の命令を保持する。
その後titleが届いても、このfieldを「現在画面が空」の意味で使用しない。
CLEARはtitle・subtitleだけを消し、action barと最後のtiming receiptを保持する。
RESETはさらに`TitleTiming::ResetToDefaults`を受信命令として記録する。
原GUIの10／70／20tickというdefaultを、実受信したtick値として合成しない。
`TitleTiming::Set`は符号付きfade-in／stay／fade-outをそのまま保持し、negative fieldも置き換えない。

これは最後に届いた表示命令の観測である。経過時間による消去、表示中かどうか、
text参照の解決や描画結果は計算しない。world変更後にも保持するtitle／tabの命令に対し、
`session`は現在のcapture境界を表す。保存済みtextを新しいworldで受信したとは扱わない。

## Tab header／footer

`TabListObservation::text`は、ひとつの完全なpacket由来のheaderとfooterの組である。
両方をdecodeしてpacket末尾を検査した後でのみ更新する。片方が欠けたpacketから新しいheaderを採用しない。
未受信は`None`であり、空componentを補わない。
これはtabの見出し観測。profile登録・latency・game mode・list entryは別の
`Client::player_list()`へ接続した。[契約](common-teams-player-list.md)を参照。

## World border

`WorldBorderObservation`はcenter、size、absolute maximum、warning delay（秒）、
warning distance（block）を個別の受信origin付きで保持する。
sizeは`WorldBorderSize::{Set, Lerp, Initialize}`を区別し、元の有限diameterと符号付き
64bit durationと元の時間単位を保持する。INITIALIZEのwarning順序は両版とも距離→時間。
1.16.1のdurationはmilliseconds、1.21.11はgame ticksで、共通`WorldBorderDuration`が単位を区別する。
`duration.nominal_milliseconds()`は20ticks/秒の換算補助で、overflowは`None`を返す。
換算値を元packetの受信値へ置き換えず、tick rate変更や実時間経過の保証にしない。
部分更新で、変更されていないcenterやwarningのoriginを新しいpacketへ移さない。

world generationが変われば古いborder値を返さず、新しいgenerationで届いたfieldだけを採用する。
INITIALIZEがまだ届かない場合にもdefault center・size・warningを補わない。
現在の補間diameter、残時間、衝突・damage・移動の許可、描画範囲は推定しない。

## 検証

元の未改変公式reader／writerで両版各21packetを生成し、titleの全命令、tabの完全pair、
borderの全命令と0／広い正数／最大値／-1のdurationを共通ledgerとadapterへ照合した。
欠損・余分なfieldは部分更新せず拒否する。別worldのborder値と切断済みClientも検査する。
原GUIとtitle handlerでCLEAR／RESETがaction barを消さないこととdefault時間を確認し、
旧版固有cacheのclear／resetとINITIALIZE warning読み順も修正した。

同じnative consumerで両版のSurvival／Creative接続へtitle／subtitle／action barと時間を送り、
borderのSET／center／warnings、CLEAR→RESET→LERP→manager終了を通す。
各fieldを透過proxyの元packetへ独立に照合し、serverのborder diameterと切断後player不在をRCONで確認する。
vanillaのこのfixtureにはheader／footerを送るcommandがないため、liveでは未受信を確認し、
そのpacketの検証は原serializerとadapter適用試験に分ける。
初回modern liveでdurationをmillisecondsとした誤りを検出し、原WorldBorder／command実装のtick単位を確認して修正した。
[固定入力と検証記録](evidence/common-ui-display-20261007.json)を参照。

この変更はB6の表示情報の観測部分。teams／player rosterは別の区切りで接続済み。特殊window、vehicle、広いmanagerと
B3〜B5、非公開A6の固定commit検証は引き続き必要。
