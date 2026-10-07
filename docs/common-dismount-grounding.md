# 下車後の共通地上継続

`survival().resume_ground(dismount_id)`と`creative().resume_ground(dismount_id)`は、
実際の下車と明示的neutralの後、同じClientで有限の地上操作へ戻る入口。
接続を作り直さず、接続時に選んだ1.16.1／1.21.11のadapterを使用する。

元の`dismount()`と`complete_dismount()`だけでは地上操作のguardを解除しない。
`DismountRecord.stage == Completed`は元の下車要求・実除外・neutral送信の完了であり、
サーバーの停止確認ではない。`resume_ground`の結果は別の`grounding`に保持する。

## 操作と証拠

元のClient・world・mode・own player・連続乗車寿命に束縛した完成済みの下車IDを使う。
現在も同じ乗車の実際の除外が保持され、元の要求より新しいown poseを受信している必要がある。
poseがpassenger除外より先に到着する順序も扱う。
現在位置がその受信poseと一致することを検査し、送信・予測位置から開始しない。

正常な立位・通常attribute・健康・flightなし・受信effectなし・既知の乾いた床と
全body／計画余白の支持を検査する。未知形状・水・姿勢変更・既知の非zero impulseは拒否する。
新版では、元の車両による中断と、その後の相対velocityが未解決である事実を保持する。
その欠測をzeroの受信sampleへ変換しない。

`DismountGrounding.declared_controller_velocity == [0, 0, 0]`は宣言したローカルの停止値。
これをseedとする2tickのreleased modelを送信し、最終の予測restと現在の床を検査する。
`grounding.motion`は開始時の受信pose、宣言した初期frame、計画、送信前のtick意図、
完全送信数、失敗理由を保持する。`Predicted`はこの有限処理の完了で、サーバーのrest ACKではない。
以後は通常の`preview_path`／`start_predicted_path`、収納等を同じhandleから呼ぶ。

## 所有と中断

一つの下車IDにつき一度だけground intentを保持し、その場で接続所有taskへ引き渡す。
呼び出しfutureを取り消しても、待機だけが取り消される。writer待ち中にも`dismount_record()`で
保持済みの意図を取得できる。未解決groundingは他のmutationを許可しない。

後続の再乗車、同じ数値IDの別寿命、位置補正、mode／world／健康／attribute／effectの変更、
遮断、不確かなwriteは後続tickを停止する。最初の失敗と実送信数を保持し、再送しない。
2tickの途中で元の車両が消滅した場合も停止する。完了後は元の車両の生存を
後続の地上runの条件にせず、受信済みの実除外と成功履歴を保持する。
2tickの成功履歴は、新しい地上runの履歴とは分けて保持する。
保存した診断値から下車IDやNativeの中断guardを復元できない。

## 検証

軽量試験は両mode・両adapterで、元のown-position codecによるposeが実除外より先に届くケース、
2tick後の新しい地上run、元のposeと新版の未解決velocity／中断情報の保持を確認する。
待機取消後の有限完了、writer待ち中の読み取り・遮断、1tick後の再乗車／車両消滅、完了後の車両消滅からの地上継続、最初の失敗の保存と
再送拒否も実transportで確認する。

実サーバーの範囲と固定入力は[共通Clientの検証記録](common-client-native-validation.md)へ記録する。
これは通常の乾いた立位へ戻る操作であり、車両物理・paddle・特殊minecart補間全体の実装ではない。
広いB3〜B6、非公開A6の固定commit検証、最終的なmain統合は引き続き必要。
