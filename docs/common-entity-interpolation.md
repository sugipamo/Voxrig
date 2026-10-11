# 明示的な生物の補間モデル

`LivingInterpolation`は、利用側が選んだ時刻・目標・衝突問い合わせの結果で、
原版の生物の位置とbody yaw/pitchの補間を計算する。受信値や移動制御とは別の、
固定サイズのモデルである。原版cowが継承する処理を比較対象とした。
種別固有のoverride、頭の角度、AI・重力・travel・押し合いは含めない。

`new(version, pose, length)`で初期状態を作る。`length`は0〜120で、原版の一般的な
生物の更新は3。パケットの種類やentity種別から回数を自動判定しない。
`set_target(before, target)`は、目標受信直前のモデル位置・角度と、解決済みの
目標位置・角度を渡す。`step(before, collision_free)`は、別に計算した移動を含む
step直前の状態で補間を1回進める。`frame()`でモデル位置・残り回数・有効な目標を読める。

| 処理 | Java 1.16.1 | Java 1.21.11 |
| --- | --- | --- |
| 同じ有効目標の再指定 | 残り回数をリセット | float/doubleのビットまで同じなら残り回数を維持 |
| 座標の更新 | 差を残り回数で割る | 差に残り回数の逆数を掛ける |
| 補間中の別の移動 | 目標をずらさない | 衝突問い合わせが許可する場合に、前回位置からの差を目標へ加える |
| 別の角度変化 | 目標角度をずらさない | 前回角度からのfloat差を目標へ加える |
| pitch | float剰余 | float剰余に加え、原版の±90度制限 |
| 長さ0の目標指定 | 移動しない | 目標へsnapし補間を解除 |

modernの有効なstepは、原版`Level.noCollision(entity, box)`に相当する問い合わせの
結果を毎回必要とする。問い合わせ位置は、現在の目標位置に
`before.position - previous.position`を加えた位置。`previous`は新しい目標を
指定したときの`before`、または前のstepの結果。同じ有効目標を再指定したときは
この基準も維持する。箱は利用側が選んだ同じ時点の生物の箱で評価する。
結果を省くと`MissingCollisionQuery`となり、モデルは変わらない。
legacyにこの結果を渡すと`UnsupportedCollisionQuery`となる。

`cancel()`は残り回数と基準を解除し、最後のモデル位置を保持する。
非有限の入力・中間値・結果は`NonFiniteState`、上限を超える長さは
`UnsupportedLength`で拒否し、失敗した操作によってモデルを部分更新しない。
入力・出力にEntityIdや送信権限を含めず、保存結果から生きた対象を復元しない。

モデルの所有者は、受信履歴を欠落なく読むこと、spawn寿命・world・接続の変更や
削除・履歴欠落でモデルを破棄すること、各パケットをどのモデルtickで適用するかを決める。
受信ordinalや履歴の`applied_after`を原版クライアントのtickへ自動変換しない。
相対teleportの解決、legacyの微小teleport位置抑制、ローカル操作権限の判定は
目標指定より前の処理であり、このAPIは代行しない。

[受信motion](common-entity-motion.md)は元のパケットの目標と各fieldのsourceを保持する。
モデルの座標で書き換えない。`derived_geometry()`も受信目標での箱のまま。
ground/boat制御の自動の接触対象選択、補間の自動スケジュール、押し合いの順序と
移動への統合は#44の残件である。

## 検証

`scripts/run_entity_interpolation_oracle.py`は公式の未変更クラスを使い、
legacyはclient JARにだけ存在する原版`LivingEntity.lerpTo`と`aiStep`を実行する。
原版の登録済みサーバーワールドをheadlessで起動し、ハーネスのcowでAIとtravelを
止めて補間段階を分離する。modernは原版`InterpolationHandler`を直接呼び、
原版の箱と`noCollision`で空間・石の障害物を評価する。
補間計算をハーネスに書き直していない。公式JARとmappingのhashを検証する。

両版それぞれ210シナリオ、合計420シナリオ・3,780操作で、座標のdoubleビット、
角度のfloatビット、残り回数が一致する。0/1/2/3/7/120の長さ、yaw境界、pitch制限、
signed zero、目標重複・差し替え、指定前とstep前の外部移動、衝突許可・拒否、
キャンセルと解除後のstepを含む。
fixtureは`data/client_api/entity_interpolation_oracle.json.gz`で、runner・ハーネスとの
hash一致もテストする。

`scripts/run_entity_interpolation_live.py`は未変更の公式両版に共通APIで実接続し、
別クライアントのpose、cowのspawn・3回の移動・削除を確認する。
実際の元の移動パケットと受信ordinal・凍結履歴を照合してから、明示的なモデルの
初期値と目標に使う。重複目標、問い合わせ欠落、計算後も受信値が変わらないこと、
明示的な解除を確認し、各版6つの原版数値シナリオも同じ接続で比較する。
両版45項目（legacy 21、modern 24）が成功し、両クライアントの中継エラーは0だった。
これは原版クライアントのtick時刻や実サーバーの接触・実行を確認するものではない。
[検証記録](evidence/common-entity-interpolation-20261011.json)には解釈済み結果とhashだけを公開し、
元のwire・NBT・JAR・classはignored `.local`に置く。
