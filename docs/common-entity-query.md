# 生物のモデル位置からの近接探索

`LivingContactIndex`は、明示的に登録したモデル時点の位置・boxから近接候補を
自動で選ぶ。`query(client_side, querying)`の結果は元の版の探索順序で並び、
`model_living_contacts`へ渡せる。モデルのkeyは数値計算用で、`EntityId`や送信権限にはならない。

検証済みの種類はplayer/cow/pig/zombie。`LivingContactIndexedBody`には種類、
モデル位置・速度・box、元のpush/spectator/local-player/team/vehicle述語を明示する。
`kind: None`はboxだけ分かる未監査の種類であり、検索範囲内で交差すると問い合わせ全体を拒否する。
teamのkeyとruleの有無が食い違う場合や、同一teamのruleが矛盾する場合も拒否する。

`upsert`は、同一区画の更新でリストの順序を保持し、区画を移ると古いリストから
除去して新しいリストへ追加する。`remove`後の追加は同じ位置でも末尾になる。
新spawnは新しいモデルkeyで登録する。packet targetを現在位置として自動登録する入口ではない。

| 探索処理 | Java 1.16.1 | Java 1.21.11 |
| --- | --- | --- |
| 水平方向の範囲 | boxのmin−2 / max＋2 | 同左 |
| 高さの範囲 | min−2 / max＋2、区画0..15へclamp | min−4 / max＋0 |
| 区画の順序 | X→Z→Yの昇順 | signed packed-section IDの昇順 |
| 区画内の順序 | 登録順のリスト | 登録順のリスト |
| model位置の区画計算 | floor(position / 16) | floor(position)を16で整数除算 |
| visibility | 読取り対象のchunk | accessibleな区画のみ |

最後の座標計算は、ゼロ直下のsubnormalでも版によって違う。
modernのpacked IDでは負のZ/Yが正のZ/Yより後に来る場合がある。
数値entity ID順やreceive ordinal順に並べ替えない。

`set_chunk_known`は、そのchunkのモデル所属が完全であることを呼び手が明示する。
空と確認したchunkも必要で、`false`は空の証明ではなく入力の欠落を表す。
問い合わせの対象範囲に未確認chunkがあると、部分的な候補を返さず`UnknownChunk`となる。
これは原版が未ロードchunkをskipして返す一部の結果を、完全な操作入力に読み替えないための制限。
modernの新しい区画は、このモデルの初期ルールとしてaccessibleになる。
原版のvisibilityが異なる場合は、次の問い合わせより前に`set_section_accessible`で明示する。
legacyにはこのvisibility分岐がなく、同じ操作を受け付けない。

保持上限は256体・256個の既知chunk。非空区画も最大256個で、空になった区画は除去する。
問い合わせは最大64個のchunk、結果は最大32体。超過時は自動evictionやtruncateを行わない。
不正な更新は既存の状態を変更しない。section packのaliasとJavaのfloor overflowを避けるため、
座標はX/Z ±32,000,000、Y ±8,000,000の範囲に限定する。

world/spawnの寿命や履歴に欠落がある場合は`invalidate()`でモデルを破棄する。
そのinstanceは再開できず、完全な入力から新しく作る必要がある。
受信履歴の適用時点とnative tick順序は#88、徒歩・ボートownerへの自動適用は#89で扱う。
この探索段階からserverの接触・実行承認や自動のClientLevel tickを主張しない。

検証用oracleは未変更の公式JAR上で、legacyの`Level.getEntities` / `LevelChunk`の
区画リストと、modernの`EntitySectionStorage`の探索・visibility処理を実行する。
48スケジュール×6段階×2版の576問い合わせを比較する。
model登録・更新・除去は自作harnessで明示し、原版packet-to-tick scheduleを実行したとはしない。
原版selectorはclient/server両分岐を切り分けて検証し、接触順にpushを適用する既存モデルとも照合する。

```bash
python3 scripts/run_entity_query_oracle.py \
  --downloads .local/climbing/downloads --work .local/entity-query/native-oracle-v1
cargo test --locked --lib client::entity::query::tests
python3 scripts/run_entity_query_live.py --accept-eula \
  --binary target/debug/examples/climbing_control_probe \
  --jars .local/climbing/downloads --compiled-sdk-revision <binary-source-commit>
```

実接続driverでは元のspawn/motion packetと履歴・寿命を照合して数値モデルへ明示的に登録する。
実際の位置更新による区画移動、除去と再spawn、未知の所属・未監査の種類・失敗した更新の拒否を検証する。
元のpacket・NBT・JAR・classはignored `.local`に保持し、公開するのは自作source、数値結果とhash。
