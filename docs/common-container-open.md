# 共通Clientでstorageを開く

`client.survival().open_container(target)`と`client.creative().open_container(target)`は、
両版で同じ`ContainerOpenRecord`を返す。受信modeに対応するhandleを選ぶ。
`chunked_survival`由来の専用APIへ切り替える必要はない。

```rust,no_run
use voxrig::client::prelude::*;

async fn activate(client: &Client) -> Result<()> {
    let target = client.survival().target_block(4.5).await?;
    if let Some(hit) = target.hit {
        let sent = client.survival().open_container(hit.position).await?;
        println!("{:?}: {:?}", sent.id, sent.stage);
    }
    // 再送ではなく同じintentの受信結果を読む。
    let current = client.survival().container_open_record().await?;
    println!("{current:?}");
    Ok(())
}
```

## 対象と事前条件

監査済みのchest、trapped chest、barrel、hopper、dispenser、dropper、ender chestに限定する。
NativeBlockStateの全propertyと、そのClientが選択した版のoutlineを照合する。
chestはtypeに応じて9×3または9×6、barrel/ender chestは9×3、hopperはhopper、
dispenser/dropperは3×3を期待する。未監査のshulkerアニメーション、entity window、
その他UIは対象外。既に受信したshulker等のstorageを観測・交換できることとは別の範囲。

健康なdry standing、default movement attributes、停止した受信姿勢、4.5以下のfirst outline、
受信されたplayer UIまたは完全送信close由来の明示的なlocal UI、選択hotbar、実受信の空の
main hand・offhand・cursor、未解決操作がないことを要求する。未知値をEmptyとしない。
通常のmodern OPEN/CLOSEでは装備とoffhandの同一worldでの最後の受信値・ordinalを保持する。
新しい受信とは扱わず、unsupported inventory packetやworld/configuration resetでは消去する。
main/hotbar/cursorはOPENで未確定となり、constructorで確認した実内容から再確立する。

ロックされたchest、上部の遮蔽物、相方の不足などでnativeが画面を開かないことはあり得る。
Voxrigは画面を捏造せず、その送信済みintentを保持する。返却timeoutや画面が来ないことは
失敗・rollback・再送許可を意味しない。

## 送信と実受信

`id`はsessionと単調増加するattemptを持つopaqueなlive identity。
I/O前にintentを保存し、main-hand activationを1回だけ送る。cursorはその境界のnative
outline点から算出する。`send.dispatched`は完全なframeが書けた場合だけtrueになる。
送信からOPEN、画面内容、cursor、active window、revisionを発明しない。

* `Pending`: 完全送信が確定していない。
* `Dispatched`: 完全送信。開いたという受信根拠はまだない。
* `ObservedScreen`: 新しいOPENを受信。期待menuと監査layoutを照合したが、結果は未完了。
* `ObservedContents`: 同じOPENのfresh full contentsと実Empty cursorを受信。
  modernでは送信以後の実global processing ACKも必要。
* `RequiresInspection`: session、mode、pose、選択・手、first outline、画面置換、cursor、
  送信不確実性等の最初の問題を保持。値が戻っても解除しない。

legacy full contentsはcursorを含まないため、OPEN以後の別の実cursor SET_SLOTが必要。
modern full contentsはcursorを含む。modern `protocol_processing`は実最高処理sequenceと
受信ordinalであり、targetや画面のACKではない。legacyには相当ACKを発明せず`None`。
どちらも新しいOPENのordinalと全slotの実受信を要求し、partial slotだけでは完了しない。

**OPEN packetにblock座標はない。** `ObservedContents`は期待する種類の新しい画面と内容を
受信した事実であり、指定targetがその画面を発生させたという因果関係・所有権の証明ではない。
titleも証明に使わない。numeric window ID再利用は新しいOPEN identityとして区別する。
別のOPENで置き換わった未完了intentは、matching contentsが後から来ても完了にならない。

`target`はI/O前のstateを保持する。`target_state`は最後に検査したreceived world cacheの
state・world revision・capture ordinalであり、そのblock更新packetのordinalとは区別する。
公式[barrel activation監査](../data/client_api/storage_activation_source.json)で確認した通常の
`open` boolean変化だけは、完全送信後に同じoutline/menuを保つ変化として扱う。
facing・名前・その他propertiesはexactのまま検査し、送信前の変化は拒否する。
このflagの受信も特定のplayerが開いたことの証明にはしない。

## 排他と取消

未完了intentは両modeの共通変更操作とnative変更操作を排他する。recordを読むだけでは
新しいactivationを送らない。serializationからlive ownerを再生成する入口もない。
`ObservedContents`の後はその実画面identityを用いてstorage SWAPやcloseを明示的に要求する。
close後の再openは新attemptであり、実際に新しいOPENを受信する必要がある。

legacyはconnection actorがwriteとownerを持つため、呼出futureをdropしても1回のwriteは
継続する。stalled writerの待機中も`container_open_record()`はintentを読み取れる。
modernはnative writerの取消契約を維持し、完全送信前に取消したintentは未確定として残す。
その後の別actorによるOPEN/full/ACKで未送信intentを完了させない。
失敗・取消後に自動retryや解放をしない。切断後も最後のrecordを読み取れる。
完了済みの受信履歴は後のmode変更や切断から書き換えない。

## 検証と残作業

同一公開consumerを両adapter・両modeで実行し、完全送信とOPEN/full/cursor/modern ACKの
境界、close後の同一numeric ID再利用、owner排他、取消、mode・cursor・画面置換の
衝突履歴を検証する。legacy actorではrevision、cancelled waiter、write failureも検証する。
[実native手順](common-client-native-validation.md)は同じClientからcreativeでopenし、
交換・close後にsurvivalで再openする。続けて両modeでbarrelをopen/closeし、
実OPEN/full/cursor/modern processingと受信cacheのopen flagを確認する。
RCONはbarrelの実open/closed状態・内容と姿勢を独立に確認し、menu所有権は主張しない。

一般block activation、非empty item use、一般click/split/shift、slot acceptance、modernの
非default components、製作・装備・entity UI・復旧は統合全体の残作業。
`Feature::Containers`はこの限定範囲のRestrictedであり、全機能の統合完了ではない。
