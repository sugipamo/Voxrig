# UI観測のcontext寿命

`scoreboard_state()`／`teams()`／`player_list()`／`boss_bars()`／`titles()`／
`tab_list()`／`world_border()`は、`context_reset_sequence: Option<u64>`を共通で返す。
これは実際に受信したcontext reset packetのordinalで、個別のUI fieldの受信とは別の事実。
初期接続と、再設定packetがない1.16.1では`None`。未受信値やnativeのdefaultを補わない。

1.21.11の`START_CONFIGURATION`を完全に読み取った時点で、次のplay listenerに
持ち越せないscoreboard・team・player一覧・boss barを破棄する。title／subtitle／timing／
clear・tab header/footer・borderも新contextで未受信に戻す。
原`Gui.onDisconnected`はaction-bar messageを消さないため、最後のaction barだけは元の
field値と元のordinalを保持する。title-familyのlast updateはその保持したmessageの実受信境界になる。
再設定から合成したREMOVE、CLEAR、RESETや10/70/20 tickの受信値は作らない。

`last_update_sequence`が`None`でも、`context_reset_sequence`があれば、
現在のcontextでは対応fieldをまだ受信していないことが分かる。
登録が消えた後のCHANGE／部分player-info更新から登録を復元しない。新しいADDが必要。
原player UUIDが同じでも、新しいprofile ADDは新しい受信ordinalを持つ。

通常のrespawnはplay listener／server registry configurationを維持する別の境界。
この実装はそこでglobal UIをresetせず、world-bound borderだけを新しいworld generationへ分ける。
通常死亡後の実respawn・fresh pose／health・接地・新しい収納操作でも、global UI／registryを保持し、
全7観測の`context_reset_sequence == None`と別Clientの独立性を確認した。
[共通respawn](common-respawn.md)を参照。chunk欠測・再接続と広いB5は後続範囲に残す。

再設定中も読み取り専用getterは欠測・空の新contextと未完了registryを返す。
`wait_until_ready()`と新しい実受信baselineを経るまで操作は許可しない。
元のscreenへは再設定中・完了後ともクリックを送信できない。
保存した観測は元の値のまま保持でき、current captureや操作IDへ再束縛されない。

## 一連の検証

未改変の公式client JAR／mappingの`handleConfigurationStart`、configuration完了時の
新listener生成、constructorのplayer-info／scoreboard初期化、`clearClientLevel`→
`Gui.onDisconnected`を照合した。game method bodyは実装へコピーしない。

公式serverの原`switchToConfig`／`startConfiguration`をserver自身のExecutor上で呼ぶ
自作試験controllerを用いる。公式bundler／Main／server JARと全codecは未改変。
controllerは試験用file requestを読むだけで、Voxrig libraryにcontrol endpointを追加しない。

```sh
CARGO_INCREMENTAL=0 python3 scripts/run_common_native.py \
  --version 1.21.11 --accept-eula --scenario reconfiguration
```

同じcommon consumerの2接続（Survival／Creative）で、UI受信・旧チェストOPEN→
実再設定／欠測観測／旧screenの拒否→実registry・play再初期化→旧screenの再拒否→
新チェストOPEN・石1個の取得／player slotへの格納／close→manager終了を通す。
新旧captureの不変性、別Clientをresetしないこと、profileと保持action barのorigin、
START／ACK／FINISHの元frame、実送信クリック2回だけと接続数2→1→2→0を独立照合する。
普通の再ログインに置き換えていないことも、LOGIN_SUCCESS計2回で確認する。
旧版は通常UI／managerの一連の操作と`context_reset_sequence == None`を確認する。
[固定入力と結果](evidence/common-ui-context-20261007.json)を参照。

この区切りはB5の新版再設定とUI寿命。広いcontext・履歴・記録・再構成・復旧、
B3／B4／B6の残差分、非公開A6の固定commit検証は引き続き必要。
