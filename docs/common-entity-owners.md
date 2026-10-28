# 徒歩・ボートの生物接触モデル

現在の資格集合は private const table から生成した次の表で確認する。
`python3 -B scripts/contact_policy_export.py --check` が集合と生成内容を検査する。
公開 geometry・packet・locality の別契約は[consumer manifest](contact-policy-consumers.md)、
原版数値 corpus の全 scenario ID は `check_contact_policy_consumers.py --check` で照合する。
Wolf は Unknown のまま、Step 6 (#128 option 3) では contact の影響を省略して制御を継続する。
[動作変更と API 移行](contact-policy-consumers.md#step-6-intentional-pass-through-issue-128-option-3)を参照。

<!-- contact-policy:selectors:start -->
| Version | Owned tick selector | Qualified species |
| --- | --- | --- |
| 1.16.1 | Chicken | chicken |
| 1.16.1 | InheritedLiving | cow, creeper, giant, mooshroom, ocelot, pig, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.16.1 | RemotePlayer | player |
| 1.21.11 | Chicken | chicken |
| 1.21.11 | InheritedLiving | cow, creeper, giant, mooshroom, ocelot, pig, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.21.11 | RemotePlayer | player |
<!-- contact-policy:selectors:end -->

## Historical audit notes (before Chicken qualification)

以下は Step C/D 当時の監査記録である。種別列挙・件数・Chicken 未資格の記述は
当時の状態を示し、現在の資格一覧ではない。現在の資格は上の生成表と
[capability audit](contact-capability-audit.md)、数値 corpus は固定 gz fixture と
[consumer checks](contact-policy-consumers.md)を参照する。副作用・限界の監査判断は保持する。


共通 `Client` の地上・held flight・ボート操作は、受信した生物の履歴を操作ownerに結び付け、版ごとの private capability table で資格化した inherited-living / RemotePlayer の通常scaleの接触を自動計算する。利用側が生物の番号を作ったり、保存したJSONから操作対象を復元する必要はない。生物との接触を除いた既存の地形、装備、乗車、実受信速度の適用、送信の検査も継続する。

## Player の接触範囲（#246）

徒歩・held flight・glide の admission／毎 physics tick は、両版で共有の
[接触スコープ](player-contact-scope.md)を適用する。model-origin／明示 tick 契約内で、
受信履歴の包絡と接触伝播により無関係と証明できる body の数値更新・地形問い合わせを省略する。
サーバーの実時間位置の証明ではない。近傍の騎乗組・未知入力・省略 tick 後の再入域は送信前に拒否する。
`max_contact_bodies` の既定値は scoped remote 256、`chunk_capacity` は受信 view に追随する Auto（上限 16,384 列）。
停止済み owner は再利用せず、原因の除去後に同じ接続で新しい `start_control` を試せる。
詳細な再開始条件と診断 API は上記の契約を参照。以下の従来の全 body 上限の記述は
独立数値 API／boat lane のものとし、player scope の body 数には適用しない。

## モデルの開始点

新しいownerは、その接続・worldに属する実際のspawn寿命と完全な履歴を読み、原版の数値constructorから受信操作を適用する。履歴を読み終えた境界が `origin_after_record` で、そこからownerが明示的に進めた回数が `model_tick` になる。既に動いていたグラフィカルなMinecraftクライアントの補間経過や、サーバーの現在位置を再現する開始点ではない。receive ordinalや経過時間を原版のentity tickへ変換しない。

履歴の保持は8,192件、1ページは最大1,024件。欠落したspawnや古い履歴を最新の位置だけで補わない。新しいownerは、完全な接触checkpointと連続した保持suffixがあれば、公開履歴の保持上限を越えていても宣言原点を作れる。checkpointがなくspawnや関係履歴を復元できない場合は、新しい接続／worldと観測が必要になる。操作中は連続した履歴を一度だけ取り込み、同じパケットを再適用しない。除去済みの古い寿命は、原版のmap/listの保持・容量履歴だけに反映する。

## 徒歩とheld flight

原版のentity更新順の中で自分の移動を実行する。自分より前に更新された生物のpushはその回の移動に入り、後のpushは次の回の速度へ残る。原版ClientLevelのliving selectorはlocal playerを選ぶため、自分のtickから全生物へ一律にpushを送る計算にはしない。

`ControlRecord.entity_contacts` は、このownerの数値frameを示す。`model_position` は補間・metadataを適用した数値feet。`model_velocity` は検証したremote AI-stepのdelta movementで、受信速度や完全な `Entity.baseTick` の流体効果、remote travel/AIを示さない。`pair_calls` は原版の早期returnを含む呼出し数で、サーバー上で何体に接触したかではない。

両版のRemotePlayerは原版constructorが `noPhysics` を設定する。従って同じ通常生物の候補になっても、継承pushから自分へ速度を足さない。1.21.11の実際の速度パケットはRemotePlayerで3段階の速度補間になる。PositionSyncが運ぶ速度は受信sampleとして保持するが、この原版handlerはderived delta movementに適用しない。

## ボート

ボートは原版の更新slotで、生物の補間モデルの箱を元のspatial検索順に取り込む。普通のliving pushはボートへ流用しない。固体衝突の `isPushable` / spectator判定と、実際の同じ乗員関係による除外を別に扱う。睡眠の早期returnは `LivingEntity.push` のfirst receiverに適用する。問い合わせ元が眠っていることだけで `pushEntities` 全体を除外しない。ボートの固体衝突から一律に除外する条件でもない。

`BoatMotion.collision_samples` の各bodyには実際の `received_position` と別に `model_box` / `geometry` がある。

| geometry | 箱の数値入力 |
| --- | --- |
| `fixed_received_target` | PR #74の固定形状の他ボート・トロッコの受信target |
| `living_interpolation` | このボートownerの履歴境界・model tick・補間feet・native float寸法 |

候補は原版のsection/list順で並ぶ。1.16.1はmetadataが箱を変えた直後にも以前の登録sectionを保ち、原版ClientLevelのtick後に移動する。1.21.11は受信callbackによるsection更新を反映する。

ボートは各tickの入力を送る前にgeometryと予測frameを保存し、完全な複数packet送信を終えたframeだけ `frames` へ追加する。入力・paddle・位置の送信完了はサーバーの承認や停止の証明ではない。最大120入力、実衝突用のbounded検索で最大32body。問い合わせ範囲は原版比較した移動上限（水平1、鉛直3）と水面へ戻る処理を覆い、物理計算はさらに実際のswept boxと原版epsilonで絞る。

## 述語と停止境界

| 入力 | 取得・扱い |
| --- | --- |
| alive | 原版constructorのモデル初期値から始め、実受信healthを適用する。実受信status/animationの未検証dispatchは停止 |
| sleeping | 原版のoptional sleeping position。pose enumだけから決めない。実受信のbedへのfeet変更と明示的なemptyも反映 |
| climbable | その生物のAI-step後のfeetと、読み込まれた実際の地形から原版の梯子・つた・trapdoor規則を検査 |
| spectator | 他playerの実登録profileと受信game modeが必要。欠落をSurvivalに補わない |
| team | playerは実profile名、生物はspawn UUIDの元のscoreboard holder。coherentな受信team snapshotで原版規則を適用 |
| passenger / vehicle | 実際の寿命付き乗員関係。own boatの乗員は衝突入力から除外。途中の変更・nested未検証関係は停止 |
| pose / baby / scale | 既存の原版寸法modelを使う。他の生物の非unit scale、未知pose/serializer、spin attackなど未検証hookは拒否。自分の地上・held flightのscale範囲は[別契約](common-player-scale.md) |

モデルには原版tick順の256 active body / 512 retained bodyの上限もある。種類不明や未検証dispatchを、遠いからという理由だけで通常生物に読み替えない。walkingでは固定車両の独自push、未監査hook、乗員travelなどを inherited-living lane へ混ぜない。ボートには既存の固定車両laneを保つ。これらの追加対応、modern相対補正・特殊トロッコの入力は #40 と個別issueに従う。

履歴欠落、部分decode、必要な地形不足、未検証hook、own correction、world/owner変更では、生物のモデルを破棄して後続frameを止める。通常の地形待ちと同じ自動再開を、このモデルへ適用しない。own sleeping / spin attack / elytra姿勢も別の操作kindを必要とする。

徒歩は保持した入力を解除する。失敗した乗車ownerは、同じ健康な実受信mount・mode・worldをまだ所有している場合に限り、neutral inputとpaddle解除を一度だけ試みる。予測位置を再送しない。`failure_release` は送信intent、完全なwrite、拒否/不確実性を分け、元の最初の失敗を保つ。切断やmount変更後は旧ownerで送らず、検査が必要な記録を残す。待つfutureの取消しは、有限planのwriterを取消さない。

crammingのサーバー側判定・damage・RNGは計算・送信しない。多数の生物の更新・pushの積算順と、SDKの保持・候補上限は別の条件である。

## 検証

`entity_owner_oracle.json.gz` は、未変更の公式client JARを先に読み込む自作headless harnessの数値結果。実際のClientLevel、LocalPlayer、Cow/Pig/Zombie/RemotePlayer、乗員を持つ原版Boat、packet handler、元のchunk/entity storageを使う。描画constructor、audio送出、passenger rideTick、LocalPlayerのGUI入力前処理は切り離し、その境界をfixtureにも記載する。

原版 fixture は両版320スケジュール・3840段階を保持し、SDKでは資格化した208スケジュール・2496段階で徒歩・ボートの位置/速度のbitsと、ボートのspatial候補順を照合する。未資格speciesの記録を通常livingの数値保証に含めない。更新が自分・ボートより前後に来る場合、複数体、速度・相対移動・teleport/sync、睡眠/解除、team、補間取消しを含む。source、runner、original JARのhashはテストで固定する。headless oracleを、完全なグラフィカルclient時計や一般mob物理の証明として扱わない。

実接続は同じ共通Clientと固定バイナリを両版公式serverで使い、実際の元packet、spawn寿命、独立RCON、停止/下車を保存する。強制下車と未検証dispatchの2場面を別runで検証し、入力解除の一度だけの送信と後続予測frameの停止を確認する。結果とSDK・バイナリ・元JARの対応は[検証記録](evidence/common-entity-owners-20261012.json)に保持する。

追加speciesは[接触dispatchの範囲](common-entity-dispatch.md)と #97/#98/#99を参照。
owner oracleのown travel比較は記録したloaded stone worldを対象とし、custom world borderを含む全地形の再資格化ではない。

自分のunit Elytra滑空は専用ownerで[資格化](common-glide-control.md)した。
他生物の滑空、追加姿勢やgliderを接触モデルへ一般化していない。

## Sheep / Creeper の追加監査（2026-10-14）

両版の監査対象に Sheep / Creeper を追加した。通常scaleの Sheep は幅 `0.9f`・
高さ `1.3f`、baby はそれぞれ `0.5f` 倍で、受信した ageable metadata の変更を
既存の reducer で反映する。Creeper は幅 `0.6f`・高さ `1.7f`。
公開 API・公開 geometry API の対象範囲は変更しない。

追加の原版入力hash、継承先、metadata、数値oracleの範囲・再実行手順は
[dispatch の追加監査](common-entity-dispatch.md)と
[監査記録](evidence/sheep-creeper-contact-20261014.json)に記載する。
未監査入力は以下の private relevance fence に従う。

## Private capabilities (Step C)

公開 API、公開 geometry の資格範囲、予測値の意味・送信保証は変更しない。
版別 private table は walking outgoing push、incoming pushability、solid collision、
geometry、metadata / status / animation callback を独立に
`Qualified / ProvenIrrelevant / BoundedUnknown / Unknown` に分類する。
既存の Cow / Pig / Zombie / Sheep / Creeper / RemotePlayer の有限な owner lane を保持し、
継承先の一致や LivingEntity ancestry だけでは新しい species を資格化しない。
[副作用監査と再生成手順](contact-capability-audit.md)、
[全登録種類の継承先・原版 method fingerprint](evidence/contact-capabilities-20261014.json)を参照。

Bat / ArmorStand はこの walking / boat contact lane に無関係と証明したため、
寸法・motion を数値 body へ補完せず native slot だけを保持する。
ArmorStand の rideable-minecart hook を walking / boat push に読み替えない。
Item / projectile を含む未監査 species は Unknown のまま PassThrough とする。
Status / animation の有限な証明は Step A の監査済み packet のみで、未知 opcode は効果を適用せず無視する。

Step B は別の保守的 envelope 証明を追加する。
[envelope の原版 owner 監査・規則・再生成手順](contact-envelope-audit.md)と
[証拠](evidence/contact-envelopes-20261015.json)を参照。
初期の有限証明は default / unmounted Spider・CaveSpider のみであり、
Unknown / BoundedUnknown の分類自体を空間的保証と解釈しない。
spawn・codec base・全補間 anchor・absolute endpoint の hull は縮めず、
最大形状と作用範囲を含める。own 前後 slot、実際の swept / step-up query、boat selector、
全 qualified remote の current / target / shifted target と固定 boat collider に交差しない
場合だけ、証明済みの無関係性を主張できる。全 remote との非交差は間接作用の最初の辺を除外する
保守的条件であり、単なる own との近距離判定ではない。

metadata / scale / equipment / status / animation / mount は有限証明を失わせる。
その寿命中の motion だけでは証明を復活させない。Step 6 では証明不能でも
PassThrough へ戻り、未知の物理効果を補完せず contact の影響を省略する。
除去・native ID 再利用・world / checkpoint 境界で再評価し、checkpoint も hull と
失効状態を保持する。真の admission / 数値処理失敗の staged frame は保持・送信しない。
履歴・provenance の破損は Permanent、受信 profile / game mode 不足は Transient のまま。


## Step D: ordinary mob qualification (2026-10-15)

The private owned contact lane additionally qualifies Rabbit, Ocelot, Villager,
WanderingTrader, Mooshroom, Silverfish and Giant in **both 1.16.1 and 1.21.11**.
Cow/Pig/Zombie/Sheep/Creeper/RemotePlayer retain their numerical regressions.
The public geometry API and owner origin, history, unit-scale, packet and mount
guards are unchanged. This does not qualify complete NPC ticks or server AI.

[The capability audit](contact-capability-audit.md#step-d-ordinary-mob-review)
records each accepted type's original owners/side effects and every reviewed
rejected type's blocker, with [dated evidence](evidence/contact-capabilities-20261015.json)
and reproducible original-client oracle commands. In particular Chicken changes
falling velocity and has modern custom baby geometry; Cat/Wolf resolve owner
teams differently by version; Fox sleeping flags and horse-family geometry
need additional modeled state. They remain unqualified. All other unclosed
reviews likewise retain Unknown (Horse's prior partial outgoing annotation
still admits nothing). No ancestry or reason-string dispatch is used.

The fixture has 320 schedules / 3,840 stages. Exact SDK comparison covers 208
qualified schedules / 2,496 stages, split equally between walking and boat;
the 1,344 unqualified raw stages are excluded from numerical claims. Baby/adult
and species-variant inputs are tested where applicable. Step 6 now permits
unqualified species near and far by omitting their contact influence. Existing
Spider/CaveSpider Step B proofs remain separate evidence of nonintersection;
proof failure falls back to PassThrough. No unqualified species is promoted.

Legacy dimension-refresh boxes retain their original minimum on equal/growing
width until a real position placement; modern refresh recenters immediately.
This state is private and checkpointed. Exact raw bounds and boat comparisons
cover sleeping/standing transitions, including legacy Silverfish seed 49 stage 7.
Step B's existing peer hull now also encloses the next recentered placement;
no unqualified provider or callback gains an envelope exemption.
