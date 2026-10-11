# 生物の明示的な接触選別と順序付きpushモデル

`model_living_contacts(version, client_side, querying, candidates)` は、呼び出し側が
選んだモデルstepで、明示した当たり判定と原版の述語を使い、候補を入力順に評価する。
`LivingContactBody` は位置・速度・box・睡眠・pushthrough・teamを持つ。
`LivingContactCandidate` はさらに同一個体、spectator、local player、同じ乗り物、
teamの同盟関係を明示する。結果にEntityIdや送信権限は含まれない。

候補は32件まで。超過、非有限値、逆向きのbounds、teamがないのにalliedがtrue、
modernの非zero legacy pushthroughを拒否する。除外される候補も先に検査し、
後続の計算で失敗した場合も部分的な結果は返さない。入力は変更しない。
boxの境界は原版のstrictな交差判定に従い、面・辺だけが接する場合は除外する。
退化したboxは勝手に膨らませない。

クライアント側の原版`EntitySelector.pushableBy`は、候補がpushableかつspectatorでなく、
`Player.isLocalPlayer()`がtrueの場合に限って許可する。
問い合わせ元のpushable・睡眠状態は、この候補選別の条件ではない。
原版team分岐は設定名から意味を推測せず、そのまま適用する。

| どちらかのteamの設定 | 同盟あり | 同盟なし |
| --- | --- | --- |
| `always`だけ、またはteamなし | 許可 | 許可 |
| `never` | 除外 | 除外 |
| `pushOwnTeam` | 除外 | 許可 |
| `pushOtherTeams` | 許可 | 除外 |

異なる設定が両方あれば、どちらかが除外する組み合わせを除外する。
`None`は呼び出し側が明示した「teamなし」。未受信のteam状態をこの値へ補完しない。
同盟関係も呼び出し側のモデル入力であり、UUIDや位置だけから推測しない。

許可した候補への原版`LivingEntity.doPush(other)`は、**other.push(this)**を呼ぶ。
そのため既存pairモデルのfirstは候補、secondは問い合わせ元になる。
睡眠判定とlegacy pushthroughも候補のものを使う。
問い合わせ元の速度には前の候補からの結果を順に引き継ぐ。
許可した接触でも、睡眠・同じ乗り物・noPhysicsなどでpushが早期終了する場合がある。
同じ入力の再計算は新しい純粋な数値計算であり、自動の一回処理にはならない。

原版oracleは、登録済みServerLevel上で原版`AABB.intersects`、
`EntitySelector.pushableBy`、cowが継承する`LivingEntity.doPush`を実行する。
クライアント分岐を切り分けるため、server thread内でlevelのclient-side flagを一時設定し、
自作のServerPlayerサブクラスがlocal-player述語だけを指定する。
各ケースでflagを復元する。候補の順序はharnessが指定する。
両版の600 batch／1,200候補を比較し、速度のdouble bitsとwrite flagも検査する。
JARを変更せず、ClientLevelの実tick・world query順序を再現したとは主張しない。

実接続driverは同じ共通Clientを両版の公式サーバーに接続する。
team設定・所属移動・削除の受信値とsource ordinalを、実際の原版パケットに照合する。
受信したteam設定を、原版oracleの明示的な数値入力へ渡して計算を検証する。
位置・box・述語・候補順序は数値モデルの入力であり、サーバー上の接触確認ではない。
元のパケット・NBT・JAR・classはignored `.local`に置き、公開するのは結果とhash。

呼び出し側は一意で完全な候補集合と原版に対応する順序を用意し、
接続・world・spawnの変更や履歴欠落で入力を破棄する必要がある。
このモデルは受信位置を現在のhitboxへ変換せず、自動の探索・tick scheduling・
cramming damage/RNG・種ごとのoverride・ground/boat ownerへの統合は行わない。
これらと自分のscale付き物理・elytra・追加の乗り物は#44の残件。
