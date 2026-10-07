# 死亡後の共通再スポーン要求

`Client::respawn().await`は、同じ接続・現在のworldで受信した非正のhealthを条件に、
原`PERFORM_RESPAWN`を一回送る。Survival／Creative handleの選択から死亡や権限を推定しない。
生存・未受信health・前worldのhealthを条件にした要求と、同じworldでの二重要求は拒否する。
正常な初期playの準備も必要。自動再スポーンや自動再試行は行わない。

`RespawnRecord`は元session、受信境界、実death healthとorigin、完全dispatch、
実RESPAWNによる新worldの受信を別に保持する。`RespawnStage`は`Prepared`／`Submitted`／
`RespawnReceived`／`RequiresInspection`。新worldの受信は因果的な要求ACKではなく、
新しいpose・health・inventoryや操作許可も意味しない。

要求の所有者は送信前に記録を公開する。呼び出しのwaiterを取り消しても所有された送信は継続する。
I/Oが失敗・不確実なら理由を保持し、同じworldへ要求を再送しない。
`Client::respawn_record()`は同期getterで、writerやpacket stateのlockを待たず、取消中や切断後も
最新の記録を取得できる。保存済みの記録と観測は過去の事実で、操作IDの復元には使わない。

要求直後に`wait_until_ready()`だけを呼ぶと、旧worldがまだReadyである可能性がある。
まず`respawn_record().received_spawn`で実新worldの受信を待ち、そのworldと同じsessionの
新しいpose・正のhealth・必要なinventoryを確認し、`wait_until_ready()`を経て操作する。
旧screen／entity／vehicleのIDを再利用せず、新しい観測から取得する。

原spawn位置は床より0.1高い場合がある。両modeの`preview_path()`／`start_predicted_path()`で
ゼロ入力を有限回送って接地へ進める。新版の空中開始は、同じ接続の所有された実RESPAWN、
そのworldのfreshな実position／ゼロvelocity／正のhealth、通常姿勢・native default属性・
既知のdry geometryを条件に、全入力がreleasedである場合だけ許可する。
予測終点の床支持とrestが成立するまで新しい地上操作へ進まない。
modelの予測終点を記録し、`position.source`は旧版の継続modelが`Predicted`、
新版の完全dispatch済み座標が`Submitted`となる。どちらも実受信位置を意味しない。
接地した位置を、元の実受信spawn poseへ上書きしない。

通常RESPAWNはconfiguration resetと異なり、server registryの所有者とglobal UI登録を保つ。
world-bound borderは新worldの実受信へ分ける。`context_reset_sequence`に合成resetを入れない。
公開APIの対応条件は`Feature::Respawn`で確認できる。

## 操作フローによる検証

```sh
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py \
  --version 1.16.1 --accept-eula --scenario respawn
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py \
  --version 1.21.11 --accept-eula --scenario respawn
```

公式vanilla・同じcommon consumerの2接続で、Survival／Creativeを順番に死亡させる。
生存要求の拒否・旧チェストOPEN→実death health→一回の要求・二重要求拒否→
実RESPAWN／新pose／health／world→有限のreleased入力で接地→旧screen拒否→新チェストの石取得・格納・close→
manager終了を確認する。原request／RESPAWN／health／poseのframeとordinal、実クリックだけ、
別Clientの独立性、UI／registry保持、保存captureの不変性を照合し、RCONでもhealth／位置／
在庫／空チェスト／接続数を確認する。JAR／codecとserverは未改変。
[固定入力と結果](evidence/common-respawn-20261007.json)を参照。

このfixtureは通常死亡、即時復活なし、inventory保持ありと明示的spawnpointを使用する。
新版のgamerule名は`minecraft:immediate_respawn`／`minecraft:keep_inventory`。
Hardcore・End creditsの生存時要求、次元移動、chunk欠測、再接続、一般的な復旧は後続範囲に残す。
自動再スポーンserverで実healthを受信する前に新worldへ進んだ場合は、過去のhealthで要求しない。
