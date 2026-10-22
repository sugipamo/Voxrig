# Resource-pack要求とheadless応答（Java 1.21.11）

configuration・playの`ADD_RESOURCE_PACK`を受信すると、同じUUIDについて
`RESOURCE_PACK_RECEIVE`の`ACCEPTED`（3）、`SUCCESSFULLY_LOADED`（0）をこの順で送る。
必須packでも入場できるようにするheadlessのprotocol応答であり、URLへのアクセス、
download、SHA-1検証、asset適用、描画は行わない。これらのstatusはassetの読込実績を表さない。
`REMOVE_RESOURCE_PACK`は対象UUID指定・全削除とも応答せず受け入れる。

## 受信した事実を読む

`Client::resource_pack_request()`は最新1件の`ObservedValue<ResourcePackRequest>`を返す。
未受信は`None`。Java 1.16.1ではこの新しい観測APIは`ErrorKind::Unsupported`となり、
既存native APIの挙動は変えない。

- `uuid`はserverから届いた16 byte（network順）、この接続内の識別子。
- `url`・`hash`・`forced`・任意の`prompt`を保持する。promptは元のnetwork NBTを持つ`UiText`。
  空hashも保持し、hashの内容やURLの到達性は検証しない。
- `source`は要求を受信した連番。configuration中も共通の受信連番を使う。
- 新しい要求が前の1件を置き換える。単一packetの上限2 MiB、文字列の既存上限とhashの40 UTF-16 code unit上限で
  保持量を制限する。全pack一覧や要求履歴を蓄積するAPIではない。
- remove・reconfiguration・切断後も最後の受信事実は残す。現在有効なpack、応答のwrite完了、
  serverの受理、assetの適用を示すものではない。別接続へUUIDを持ち越さない。

`EventKind::ResourcePackRequested { uuid }`をconfiguration・playの両方で記録する。
`events_after`／`wait_for_events`の既存4096件の保持範囲・cursor契約を使い、
eventの受信連番と観測の`source`を照合する。複数要求が届いた場合、古いeventに対応する
完全な要求はすでに置き換わっている場合がある。重複UUIDも新たな要求として応答・通知する。
不正・途中で切れた要求は応答や要求の保存・通知を行わず、既存のprotocol受信失敗処理に従う。

Voxrigの対象はoffline-modeサーバー（`online-mode=false`、`enforce-secure-profile=false`）。
Microsoftアカウント認証・online-mode暗号化、および署名付きchat・commandはスコープ外。

## 検証

構築packetのunit testで両フェーズのUUID別応答順、任意prompt、forced、空hash、
複数要求・重複UUID、remove、reconfiguration、不正／truncated packetの非commitを確認する。
実接続用の共通API probeは`examples/resource_pack_probe.rs`。
使い捨ての公式1.21.11 serverに`resource-pack=https://example.invalid/pack.zip`、
40桁の`resource-pack-sha1`、`require-resource-pack=true`、`online-mode=false`を設定し、
`VOXRIG_PORT=<port> cargo run --locked -j2 --example resource_pack_probe`で要求・eventと
play到達後35秒間の接続維持を検査する。実行結果は[検証記録](evidence/resource-pack-20261022.json)を参照。

これは[Issue #46](https://github.com/sugipamo/Voxrig/issues/46)のresource-pack protocol面だけを扱う。
