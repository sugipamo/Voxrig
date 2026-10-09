# 受信した地図

`Client::map_observation(native_id)`は両版で`Option<MapObservation>`を返す。
地図itemのslotやcomponentとは別に、map packetのscale、locked、icon一覧、pixelを読む。
未受信のIDは`None`。切断後も最後のcaptureを読める。

`MapIdentity`は元のconnection/version/world generation、native map ID、最初の受信番号を
持つ。world変更とreconfigurationは共通cacheを退役させる。保存したcaptureは元の
identityとcoverageを保持する。同じmap IDを新しいworldで再受信すると別のidentityに
なる。packetには地図のdimension、中心座標、item slotがないため、これらを補完しない。
これはworldごとの受信観測cacheであり、ゲームの地図asset全体の寿命を再現する機能ではない。

`pixel(x, y)`は128×128内の最後の受信色と、そのcellを覆った元packetのsourceを返す。
初めて受け取った部分rectangleの外は`None`で、色番号0の受信とは別。重なる部分更新では
触れたcellだけsourceを更新する。headerやiconだけのpacketでpixelのsourceを刷新しない。
`known_pixels()`で明示的に受信したcell数を読める。

modernのicon省略は以前の一覧とsourceを保持し、空一覧は一覧を空に更新する。
未受信の一覧は`None`。legacyは毎packetに一覧がある。legacyだけの
`tracking_position`はmodernで`None`。iconの型はlegacy enum ordinal／modern registry IDを
区別し、static name fallbackを使わない。signed X/Y、元rotation byte、optional nameを
保持する。`rotation()`はnativeと同じ下位4bitを読む。nameは元のJSONまたはunnamed NBTの
`UiText`で、描画・実行権限やworld位置へ変換しない。

共通cacheは1world最大128map、1一覧4096icon、name合計1 MiB。
pixel/source arrayは各mapで固定16384cell。保存したcaptureはArcで共有し、後の更新は
copy-on-writeする。payload・rectangle・color length・trailing dataと容量を検証してから
cacheへ適用し、上限や不正packetで部分更新しない。保持済みmapの更新は容量上限でも受け取る。
legacy nativeの既存`ConnectionOptions.max_maps`指定も維持する。

再現用の公式実接続driverは`scripts/run_map_context.py`。実際に空の地図を使って公式
サーバーにmap IDを発行させ、元packetのheader・icon・全pixel/sourceを独立に再構成し、
公式サーバーが保存した地図のcolor arrayとも照合する。world変更、同じIDの再受信、
保存したcaptureと切断後の読取も確認する。未受信の部分coverage、icon省略／空、
不正更新と容量上限は別のwire fixtureで検査する。
modern公式サーバーは別dimensionで持った地図をこのfixtureでは送らないため、欠測のまま
保持し、元のdimensionに戻った後で同じIDの再受信を検査する。以前のworldのpixelを
補完しない。公式両版の結果とSDK/JAR hashは
[保存した検証結果](evidence/common-map-context-20261009.json)に記録する。原packetと
保存された地図の画素はローカルの検証artifactに保持し、公開証拠は結果とhashのみ。
