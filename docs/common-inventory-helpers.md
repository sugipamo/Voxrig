# inventory の補助（`compact_inventory`・`fill_crafting_grid`・`craft_once`）

共通の `Survival::click_inventory`（通常の左・右クリック）と `transfer_crafting_result` を組み合わせた補助です。両版とも同じ手順で動きます。各クリックは、その記録が `Pending` を抜ける（受信した結果がそろう）まで最大 3 秒待ってから次へ進みます。`ObservedClicked` 以外で終わったクリックはエラーになり、そこで止まります。

## `compact_inventory()`

1. cursor に持っている stack を収納します。同じ item と data で空きのある stack を先に、なければ最初の空き slot を使います。
2. 収納の末尾から先頭へ（main 9..35、次に hotbar 36..44 の順）見ていきます。前方に同じ item・data で空きのある stack があれば、拾って前方へ詰めます。

player 画面でも、開いている container の player 部分（検証済みの layout がある場合）でも動きます。戻り値は全クリックの記録です。

## `fill_crafting_grid(ingredients)`

開いている crafting grid（player の 2×2、または作業台の 3×3）に手で材料を並べます。

- `ingredients` は行優先で、grid の大きさと同じ長さにします。`Some("oak_planks")` は `minecraft:` を補います。
- まず grid に残っている物を収納へ戻します。
- 各材料について、収納の stack を左クリックで拾い、grid に右クリックで 1 個置き、残りを元の slot に戻します。
- 最後に、受信した結果 slot を最大 2 秒待ちます。戻り値は `CraftingFillRecord { clicks, result }` です。

## `craft_once(ingredients)`

`fill_crafting_grid` の後、結果を `transfer_crafting_result`（shift で収納へ）で 1 回取り、その記録の結果を待ちます。結果が出なければエラーです（材料は grid に残り、次の fill で戻ります）。戻り値は `CraftOnceRecord { fill, take }` です。

## 制限

- 同じ item で data（enchant・名前など）が違う stack はまとめません。
- 材料は item 名で探します。data は区別しません。
- recipe book（`place_recipe`）は使いません。サーバーの判定は受信した結果 slot で見ます。

## 公式サーバーでの確認

`examples/inventory_helpers_probe.rs` で両版を確認しました。

- 散らばった板材（5・7・60）と丸石（3・2）を、それぞれ 64+8 の 2 slot と 5 の 1 slot にまとめました（5 クリック）。
- player の 2×2 で棒 4 本を作りました（板材 −2）。
- 作業台の 3×3 で木のツルハシを作りました（板材 −3、棒 −2）。
