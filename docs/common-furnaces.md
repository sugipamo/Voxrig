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

1.21.11の同じconstructor確認済みopeningでは、`transfer_inventory(source, slot)`で
input/fuel/outputと付属player slotのQUICK_MOVEも1回送れる。input/fuelからはplayerへ正順、
outputからは逆順に移す。付属playerからは実受信recipe property setによる材料判定を先に行い、
次に既存の実受信tag照合を共用した燃料判定、それ以外はmain/hotbar間の正順転送になる。
材料兼燃料はinputが満杯でもfuelへ迂回しない。空bucketはfuel slotへのPICKUPでは容量1だが、
QUICK_MOVEの燃料ではなくmain/hotbar間を移る。outputの回収は実slot変化だけを予測し、XPを証拠にしない。

全39slotの実受信baselineとrevisionを要求する。付属playerからの転送ではrecipe property set未受信・
宣言欠測を拒否し、燃料判定に必要なtag未受信・不一致でも送信しない。転送中のproperty/tag所有変更、精錬によるinput/fuel/outputの
予測外変化はRequiresInspectionへ保持する。sourceと変化した全destinationのfresh受信が揃うまで
完了しない。自動消費後に値が戻っても記録を成功へ戻さず、クリックを再送しない。
完了済み記録は後の精錬で書き換えない。1.16.1の炉QUICK_MOVE、SWAP拡張、recipe選択・数量計画・
1.16.1の燃焼進捗観測、XP予測、独自datapack燃料や広いwindow統合は残る。
3種の炉で両modeの一連操作を実サーバー検証した。constructorとslot規則は
独立した公式JAR試験でも検証し、各種の全レシピを実サーバーで通した証拠とは区別する。

QUICK_MOVEの原版fixtureは`export_furnace_transfers.py`で生成する。既存transfer exporterと同じ
`--downloads`、`--modern-classpath-file`、`--runtime-output`を指定し、`--check`で再照合できる。
未変更の公式1.21.11 JAR、vanilla recipe/tag loader、実FuelValues、実menuとUPDATE_RECIPES codecを使う。
両mode・3種・7source境界・data/held cursor・満杯/部分mergeなど5,880件の全39slot結果を保存する。
constructorのSimpleContainerにはblock entityのrecipe使用履歴がないため、このoracleはXP生成や
自動精錬を証明しない。packet試験と使い捨て公式serverの実接続検証を別に行う。

実接続検証は[2026-10-23の記録](evidence/common-furnace-transfer-20261023.json)に保存した。
両mode・3種の炉で28回のQUICK_MOVEを行い、燃料・材料投入、実精錬後のoutput回収、fuel回収、
通常炉の材料兼燃料のinput優先とinput回収に成功した。全changed slotのfresh受信、元packet、
RCONの実在庫を別々に保持する。tick freezeで転送と自動精錬を分け、精錬後も完了履歴が
書き換わらないことを確認した。進行中消費との競合や部分転送は別のpacket/oracle試験で検証する。
serverは一時ディレクトリのコピーを使用し、検証後に正常停止した。

1.21.11の燃焼・cook propertyは、別の読取専用`Client::screen_properties()`で実受信値とordinalを確認できる。
slotや精錬完了をこの値から補完しない。詳細は[opening付きproperty観測](common-container-observation.md#opening付きproperty観測)を参照。
