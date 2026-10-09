# 共通のかまどスロット操作

`Client::furnace_state()`は、両版で同じ`FurnaceObservation`を返す。
実際に開いたかまど・溶鉱炉・燻製器のconstructorで確認した39slotと、player slotの対応を使う。
`FurnaceSlot::{Input, Fuel, Output}`からスロットの観測とクリック位置を取得でき、
利用側でnative menu IDやplayer offsetを分岐する必要はない。
未受信のslotはNoneのまま。入力に置けることと、その材料のレシピがあることは別である。

```rust,no_run
use voxrig::client::prelude::*;
async fn deposit_carried_fuel(client: &Client) -> Result<()> {
    if let Some(furnace) = client.furnace_state().await? {
        let (source, slot) = furnace.slot_source(FurnaceSlot::Fuel);
        // 既存のcursorに保持した燃料を、受信済みSurvival modeで投入する。
        let submitted = client.survival()
            .click_inventory(source, slot, InventoryClickButton::Left).await?;
        // 完了の判定にはinventory_click_recordの実source/cursor受信を確認する。
        println!("{:?}", submitted.stage);
    }
    Ok(())
}
```

開閉には既存のmode handleの`open_container`／`close_container`、通常操作には
`click_inventory`のPICKUPを使う。受信mode・同じopening・source/cursorの元受信・revisionを
送信直前にも検査し、送信結果とslotの実変化を区別する。古い画面IDへのクリックは拒否する。
画面closeを送った後の古い観測から、活きたかまどの状態を復元しない。

燃料のmayPlaceとcapacityは公式JARのvanilla tag／FuelValuesと実Slotへ照合して固定する。
空バケツは燃料slotで1個に制限する。投入するitemについて、native燃料処理が参照するtagの
受信membershipが固定vanillaの規則と一致することも確認する。tag未受信・宣言欠測・
対応規則の変更は黙って同じ燃料と解釈せず拒否する。custom fuel/datapackの対応は残る。
結果slotは投入を拒否し、取り出しと同一itemのcursorへの結合をnative規則で扱う。
XPや実績callbackの結果をslot予測へ置き換えない。

精錬そのものはserverが実行し、完了は新しいoutput受信で確認する。fuel消費やcook timeを
clientが進めず、経過時間だけで完成と判断しない。入力がすでにある場合、投入したfuelが
slot受信前に消費されることがある。共通PICKUPの予測どおりの実source/cursorを確認できなければ
RequiresInspectionとなり、自動再送しない。代表workflowではfuelを先に置き、その後に材料を置く。

この段階の対象は基本slot操作。recipe選択・数量計画・燃焼進捗の共通観測、特殊QUICK_MOVE／
SWAP、XP予測、独自datapack燃料や広いwindow統合は残る。
通常のかまどで両modeの一連操作を実サーバー検証し、溶鉱炉／燻製器のconstructorとslot規則は
独立した公式JAR試験で検証する。各特殊レシピを実サーバーで通した証拠とは区別する。
