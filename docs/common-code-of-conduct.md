# Code of conductの自動accept（Java 1.21.11）

configurationの`CODE_OF_CONDUCT`（clientbound `0x13`）を受信すると、本文を保存し、
`ACCEPT_CODE_OF_CONDUCT`（serverbound `0x09`、payloadなし）を自動送信する。
呼出側による手動承認待ちは設けず、headless clientのprotocol応答としてconfigurationを進める。
接続はそのサーバーのcode of conductを自動acceptする方針であることを前提にする。

## 受信した事実を読む

`Client::code_of_conduct()`は最新1件の`ObservedValue<String>`を返す。未受信は`None`。
Java 1.16.1は`ErrorKind::Unsupported`を返す。この追加APIはfeature `native`を必要としない。

- 本文はVarIntのbyte長に続くUTF-8文字列1個。NBTのtext componentではない。
  改行・Unicodeを受信したまま保存し、最大32,767 UTF-16 code unitsを許可する。
- `source`はconfiguration中も共通の受信連番（`ValueSource::Received { sequence }`）。
  `EventKind::CodeOfConductReceived`の`receive_sequence`と照合できる。
- 新しい要求（同じ本文の再送を含む）ごとにaccept・通知し、保持する本文は最新1件で置き換える。
  eventは既存の4096件の保持範囲とcursor契約を使う。古いeventに対応する本文は、
  すでに置き換わっている場合がある。
- reconfiguration・切断後も最後の受信事実を保持する。別接続へ持ち越さない。
  本文・eventはacceptのwrite完了やserverの受理を示さない。play到達は別に確認する。
- 不正UTF-8、途中切れ、長さ超過、余剰byteのあるpacketは、本文保存・通知・acceptを行わず、
  既存のprotocol受信失敗処理に従う。

Microsoftアカウント認証・online-mode暗号化、および署名付きchat・commandはスコープ外。
対象は`online-mode=false`、`enforce-secure-profile=false`のサーバー。

## Protocolの根拠と検証

公式1.21.11 server JAR（SHA-1 `64bb6d763bed0a9f1d632ec347938594144943ed`）と
[Mojang公式server mappings](https://piston-data.mojang.com/v1/objects/5621e9253f05fd57872bbe7f8ddf5f9a7d525955/server.txt)を
`javap -c -p`で照合した。`ClientboundCodeOfConductPacket`（`aci`）のcodecは
`ByteBufCodecs.STRING_UTF8`（`aam.p`、32,767 UTF-16 code units）、
`ServerboundAcceptCodeOfConductPacket`（`acr`）は`StreamCodec.unit(INSTANCE)`。
`DedicatedServer.readCodeOfConducts`は`codeofconduct/<language_code>.txt`をUTF-8で読み、
行を`\n`で連結してformatting colorを除いた文字列を使う。この検証fileの末尾の改行は送信本文に含まれない。
Voxrigのsettingsは`en_us`を送る。

unit testは構築packetで空文字列・複数行・Unicode・重複要求・再configuration・不正packet・
UTF-16長の境界を検査する。公式サーバーで捕捉した本文と空のaccept応答も固定bytesで照合する。

使い捨ての公式1.21.11 serverで`enable-code-of-conduct=true`、`online-mode=false`、
`enforce-secure-profile=false`を設定し、`codeofconduct/en_us.txt`を用意する。
`examples/code_of_conduct_probe.rs`は期待本文、受信event、play到達、35秒間の接続維持と
切断後の本文保持を確認する。

```bash
VOXRIG_PORT=<port> VOXRIG_CONDUCT=$'Be kind.\n建築を大切に 😀' \
  cargo run --locked -j2 --example code_of_conduct_probe
```

実接続のpacket・結果・公式JARのhash・cleanupは[検証記録](evidence/code-of-conduct-20261022.json)を参照。
これは[Issue #46](https://github.com/sugipamo/Voxrig/issues/46)のcode-of-conduct protocol面だけを扱う。
