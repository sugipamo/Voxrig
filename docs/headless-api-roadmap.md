# Headless client API拡張ロードマップ

この文書は旧Java 1.16.1 native APIの拡張記録です。現在の両版共通`Client`の完了表は
[利用クライアント向け網羅表](downstream-client-coverage.md)と[能力表](common-capabilities.md)を参照してください。

MineflayerやAzaleaとの比較から、pathfinding・計画・意味判断を除外し、protocol-facingな身体・sensorとして必要な公開面を整理します。

## 完了

- block/entity raycast、crosshair target、視線、reach判定
- 採掘可否、harvest可否、tool効率、予測tick
- 設置対象のload、reach、視線、player cell交差
- block、inventory、entity revision待機
- chunk一覧、load待機、低コピーsnapshot
- biome、sky/block light、heightmap NBT、block entity NBT
- map itemのicon、部分pixel更新、128×128状態
- 同一manager内のchunk section共有とcopy-on-write
- fluid、眼の水没、climbable、特殊block接触、足元block
- passenger追跡、mounted input、boat paddle、dismount
- 村人取引状態と選択、enchant、anvil、beacon、sign、手の交換
- client settings、brand/custom payload、接続timeout設定
- scoreboard、team、boss bar、title/action bar、tab header/footer、world border
- resource pack要求/status応答、transaction同期済みtab completion
- particleとworld eventの構造化event
- recipe book unlock/settingsとstatisticsのrevision付き状態
- advancementのdisplay・criteria・requirements・progress状態
- book編集、明示slot装備・移動、選択stackの指定個数drop
- explosion、world view、camera、attach/leash、stop sound、NBT query
- command tree、server recipes/tagsと全clientbound packet IDの明示処理
- block state範囲query、複数chunk待機、block値変化・placement結果待機

## 検証結果

- Java 8 / 公式Minecraft 1.16.1 / offline-mode / Peacefulで初期同期を検証
- command tree 20 nodes、144 tags、859 recipes、121 chunksを実データから取得
- 2 Botsで同一chunk sectionの`Arc`共有を確認（60参照、copy-on-writeはunit testで確認）
- 1 Bot × 20秒と10 Bots × 60秒の移動試験を完走し、切断・位置補正ともに0
- 全unit test、全exampleのcompile、doc test、warningをerror扱いしたClippyが合格

再実行可能な初期同期・共有検査は`examples/api_surface_probe.rs`、耐久結果は
repository版の`reports/api-expansion-smoke.md`と
`reports/api-expansion-10bot-60s.md`に保存しています。

当時はvehicle input/poseのprotocol面を対象としていました。現在は共通APIでボートの有限入力・物理予測と
server駆動のトロッコを提供しています。[乗り物の現在の制約と検証](common-vehicles.md)を参照してください。
特殊containerのnative parser検査は、共通APIや個別場面の実server検証とは区別します。

## 引き続き責務外

- pathfinding、目的地選択、障害物回避戦略
- 自動道具・装備選択、自動食事、自動resource gathering
- crafting材料計画、建築計画、combat戦術
- 自動再接続方針、複数agentの役割分担

責務内APIは「clientが知っている事実」「指定された入力の送信」「server結果との同期」までとし、その事実から何を行うかは外部controllerが決定します。
