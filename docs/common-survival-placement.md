# 共通Survivalの設置

`Client::survival().place_cube(support, face)`と`placement_record()`は、
1.16.1と1.21.11で共通の`PlacementRecord`を返す。通常の利用側に版分岐は不要。
modern専用の`place_survival_cube`等も維持する。

```rust,no_run
use voxrig::client::prelude::*;
# async fn place(client: &Client) -> Result<()> {
let survival = client.survival();
// 利用側が設置場所・権限・材料を選ぶ。対象がなければ送信しない。
let Some(hit) = survival.target_block(4.5).await?.hit else { return Ok(()); };
let attempt = survival.place_cube(hit.position, hit.face).await?;
// 送信の完了と結果は別。Pendingでも同じ操作を再送しない。
let diagnostics = survival.placement_record().await?;
# let _ = (attempt, diagnostics);
# Ok(())
# }
```

## 前提と送信

健康な通常立位・接地・乾いた既知geometry・通常属性・受信済みsurvival modeとown poseを要求する。
空のcursorと選択中のdefault passive cube stackには実受信の根拠が必要。
player screenへの操作基準は、実受信のwindow 0、または同じ接続・世代・openingに結び付いた
完全な`close_container`送信とする。vanillaがcloseをechoしなくても、収納・通常製作の後に
共通設置へ進める。`initial.inventory.player_screen`にその基準を保持し、受信windowやscreenを
window 0へ書き換えない。再OPEN・ID再利用・不完全なcloseはこの基準を失わせ、
送信前は拒否、設置の実受信待ちでは競合を保持する。modern版固有のchecked設置契約は維持する。
材料は共通dry modelのcubeからgrass blockを除いた範囲。NBT/components付き材料、工具、液体、
未ロード・未知形状はこの入口では扱わない。legacyのgeometryは監査済みのair/12種のcubeに限定する。
effect更新がないことを、serverのeffect一覧が完全に空である証明にはしない。

支持blockと面は4.5block以内の最初のnative outlineと一致し、隣接対象は受信済みairかつ身体の外側であることを検査する。
`cursor`は実際のfirst hitから計算した支持block内の座標で、面の中心を仮定しない。
`initial`はadapterの同じ境界でI/O前に取得したplayer/inventory capture。
`PlacementId`は元sessionとattemptに結び付いたopaqueな識別子で、Deserializeを持たない。
保存したJSONを操作能力として復元しない。

`send.after_sequence`はI/O前の実受信境界、`dispatched`は完全なcommand frameの送信を示す。
legacyは0x2dのmain hand・packed support・face・3つのfloat cursor・inside flagを送る。
modernだけが持つinteraction sequenceをlegacyへ追加しない。
legacyの`interaction_sequence`と`processing`は`None`である。

## 結果と次の操作

| stage | 意味 |
| --- | --- |
| `Pending` | 必要な実受信が揃っていない。再送しない |
| `ObservedPlaced` | 完全送信、新しい対象block、ちょうど1個の材料消費を照合。modernは新しい実processing ACKも要求 |
| `RequiresInspection` | 最初の競合・前提喪失・不確実な送信を保持。後で復元しても消さない |

`target_receipt`は指定cellの新しい実block update、`material_receipt`は選択slotの新しい実更新である。
一方だけ、無関係なblock更新、cacheの一致、ACKだけでは完了しない。
材料は同じitem ID・名前・dataを持つstackのcountが1減ることを照合し、最後の1個なら明示的なEmptyを要求する。
別actorによる設置・材料の変化と区別できるとは約束しない。
modernの`processing`は実際の処理sequenceとACK packet ordinalで、設置成功そのものではない。

未解決の間は視点・選択・移動・二重設置等を拒否する。
legacyはconnection actorが通常操作を排他し、autonomous physicsを停止する。
実受信が揃った後、`placement_record()`が同じattemptのgateを解放する。解放packetは送らない。
modernは既存のnative placement guardで同じ前提と結果を検査する。
完了後は新しい場所で次の操作を明示的に開始でき、その場のmode・材料・身体・支持・対象を再検証する。
採掘のair/ABORTによる継続とは別の契約であり、未解決の採掘・移動を設置で解除しない。

完了した対象・材料の受信記録は履歴として保持し、後の現在world/slotと分ける。
閉じた接続でも診断を読み出せる。modernのnative専用操作へ移った場合も直前の共通診断を残す。

## 取消・競合と検証

legacyの保持したcommandはconnection-owned taskが一度だけ実行し、waiter取消で再送しない。
modernのwriter取得前取消では未送信intentを保持し、その後に対象と材料が変化しても送信済みにしない。
partial write・接続終了・一時的な足場喪失・pose/selection/cursor/material変更・chunkの置換等を診断に保持する。
取消・timeoutをrollbackや設置されなかった証明として扱わない。

同じconsumerのpacket試験で、両版の前提capture、freshな対象/材料、競合拒否、取消、一度性、
完了後の次の設置を検査する。modernでは古いACKのordinalを完了根拠にしない。
legacyの特殊Set Slot window -2は、公式1.16.1のInventory送信とInventoryMenu構成へ照合し、
raw inventory番号から共通player screen番号への変換を修正した。通常のwindow番号・cursor・受信ordinalは保持する。

公式vanilla両版では採掘試験後の新しい接続でdirtを設置し、Clientのfresh target/material受信と、
別経路のRCONによる対象dirt・材料3→2・位置不変を照合する。両JVMは順番に実行し正常終了を要求する。
再実行方法・証拠は[共通native検証](common-client-native-validation.md)を参照。
任意形状の設置・container操作・共通fresh recoveryは後続作業に残る。
