# 共通team・player一覧観測

`Client::teams()`と`Client::player_list()`は、setupで選んだ1.16.1／1.21.11の
packetを同じ公開型で返す。Survival／Creative以外の受信game modeでも観測できる。
送信・描画・認証や操作権限の生成は行わない。

## Teams

`TeamsObservation::teams`は実ADDを受信したteamを名前順で保持する。
`TeamParameters`はdisplay・raw friendly flags・visibility・collision・color・prefix・suffix。
ADD／CHANGEではひとつの完全なparameter組を受信するため、組全体にそのpacketのoriginを付ける。
visibility／collisionと17種類のcolorは共通enum、textは元JSON／NBTを保持する`UiText`。
flag byteの未知bitも保持し、collision ruleを物理計算や操作許可へ読み替えない。

memberは`ObservedValue<String>`で、実ADD／JOINのoriginを個別に持つ。
これはscoreboard holder名で、online playerだけではない。
一人のholderは一つのteamに属し、別teamへのJOINで前の所属を除く元の規則を反映する。
残るmemberのoriginをmetadata更新や別holderの移動で更新しない。
`last_members_update_sequence`はLEAVEと他teamへの移動による除去も含む。
REMOVEはteamと所属を除き、global `last_update_sequence`へ実packetを保持する。

未宣言teamのCHANGE／JOINからteamを合成しない。
元のscoreboardが旧版の重複ADDを拒否し、新版は既存teamを保持するため、その違いを維持する。
所属と異なるteamのLEAVEや同じholderの重複LEAVEは、部分変更せず拒否する。
packet全体と適用条件を検査してから更新する。
保持上限はteamとholderを合わせて4096、保存するnative text/name encodingは16MiB。
旧版固有cacheも同じdecoderへ接続し、所属移動・未宣言CHANGE・原子的decodeを揃えた。

## Player一覧

`PlayerListObservation::entries`は実ADDの登録をUUID順で保持する。
`PlayerProfile`はnameと全property・optional signature文字列を保持する。
profileのUUIDはentity IDではなく、listへの登録を近くのentity spawnと扱わない。
未登録UUIDの部分更新からprofileを作らない。REMOVE後はentryを除き、global ordinalを保持する。

game mode・latency・display override・listing・list order・hat・chat sessionはfieldごとの
`ObservedValue`を持ち、変更されなかったfieldは元のordinalのまま残す。
ADDで登録を置き換えた場合、そのADDで届かなかったfieldは未受信に戻し、defaultを補わない。
各Optionの外側の`None`は未受信またはその版にfieldがないことを示す。
`display_name`と`chat_session`の内側の`None`は、実packetでoverride/sessionがないと受信したこと。
`game_mode`の内側の`None`は、旧版の実NOT_SET（-1）で、通常のSurvivalではない。

`PlayerListing::LegacyEntry`は旧版の実ADDによる登録で、booleanのlisted packetではない。
新版の`Listed { listed }`は実UPDATE_LISTED flagで、登録が残ってもfalseになり得る。
`is_listed()`はこのnative規則の解釈を返す補助で、描画結果や完全なonline catalogueを保証しない。
旧版にないorder／hat／chatを0／true／空データで埋めない。
latencyは元の符号付き整数で、最新RTTの測定値には置き換えない。

`PlayerChatSession`はUUID・符号付きexpiry epoch milliseconds・encoded public key・signature bytesを
そのまま保持する。property signatureとともに、暗号的検証・期限判定・認証成功を表さない。
保存上限は4096profile、1024property/profile、native encodingの合計16MiB。
新版の空間player trackerもprofile count/property countをこの範囲へ揃え、
UPDATE_LIST_ORDER（bit 6）をvarint、UPDATE_HAT（bit 7）をboolとして読む誤りを修正した。
空間entity観測の既存の上限やphysicsまで拡張したとは扱わない。

## Captureと検証

team／profile登録はconnection内の通常play中のworld変更をまたいで保持する。
各captureの`session`は現在のworld境界で、前worldの受信ordinalを新worldでの受信へ付け替えない。
切断済みClientからの取得はerror。saved observationをlive接続や操作IDへ戻さない。
新版の再configuration前後のcache寿命・再登録は未検証で、B5の後続フローに残す。
通常playの結果から再configuration後の登録の有効性を保証しない。
`Feature::Teams`／`Feature::PlayerList`で対応条件を確認する。
`tab_list()`はheader／footer、`entity_motion()`はspawn寿命と空間情報を扱い、これらと混同しない。

原codec計109例、元scoreboardの所属移動／重複作成／誤LEAVE、全truncationとtrailing field、
両adapter経由の全packet適用を検証する。
同じnative consumerで2つの接続へteam作成・metadata更新・offline holder参加・所属移動・
LEAVE／REMOVEを配信し、peerのSpectator→Creative更新、peerの個別切断による実roster REMOVE、
残るClientのmanager終了まで通す。各fieldのordinal／元bytesと、独立RCONの所属・mode・接続数を照合する。
[固定入力と結果](evidence/common-teams-player-list-20261007.json)を参照。

この変更はB6のteam／player一覧部分。描画、特殊window・vehicle・広いmanager、
B3〜B5と非公開A6の固定commit移行結果は引き続き必要。
