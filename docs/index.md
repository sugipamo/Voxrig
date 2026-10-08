# Voxrig ドキュメント

`voxrig`は、外部の頭脳から直接操作するMinecraft Java Edition向けheadless clientクレートです。

`Bot` の既存説明は1.16.1専用です。共存する1.21.11観測アダプタは
[移行計画](dustroute-integration.md)、[対応範囲と検証](version-adapter-validation.md)、
[ピストン通知と階段の未同期](piston-client-update-prerequisite.md)を参照してください。
追加実装は[クライアント側ピストン処理](client-piston-reconstruction.md)、
レビュー方針は[PR準備](pull-request-preparation.md)にまとめています。

## 利用者向け

- [共通trait設計と版別対応表](version-adapter-trait.md)（版間API差の整理の正本）
- [共通APIの能力表](common-capabilities.md)
- [版ごとの定数表](version-tables.md)
- [汎用物理の設計メモ](physics-design.md)
- [共通chat・command](common-chat.md)
- [共通の待機API](common-waits.md)
- [共通のchange通知（event）](common-events.md)
- [共通のblock検索とraycast](common-blocks.md)
- [共通のentity現在状態](common-entities.md)
- [共通の継続操作（キーを押し続ける移動）](common-control.md)
- [共通のアイテム使用](common-item-use.md)
- [共通event streamの設計メモ](event-stream-design.md)
- [Client共通化のロードマップ・現在地](client-unification.md)
- [公開client APIの設計](public-client-api.md)
- [0.2 client APIへの移行](client-api-migration.md)
- [導入と最初の接続](getting-started.md)
- [対応機能と制約](capabilities.md)
- [公開API](api.md)
- [Java 1.21.11のサバイバル所持品交換](survival-inventory.md)
- [Java 1.21.11の静止・接地判定の基盤](survival-standing-context.md)
- [Java 1.21.11の採掘完了・中断の実機比較](survival-mining-comparison.md)
- [Java 1.21.11の送信中断と切断後の履歴](survival-outbound.md)
- [Java 1.21.11の限定サバイバル採掘と継続境界](survival-mining.md)
- [Java 1.21.11の採掘接続の退出確認と再接続](survival-mining-retirement.md)
- [Java 1.21.11の同一プロフィールでの採掘復旧](survival-single-profile-recovery.md)
- [Java 1.21.11の明示的な予測移動契約](survival-predicted-motion.md)
- [Java 1.21.11の共通ロード完了処理](survival-interaction-loading.md)
- [Java 1.21.11の通常配置と材料の受信確認](survival-placement.md)
- [共通Survivalの設置と材料の受信確認](common-survival-placement.md)
- [共通Clientの在庫交換](common-inventory-swaps.md)
- [共通Clientの通常クリック](common-inventory-clicks.md)
- [共通ClientのShift転送](common-inventory-transfers.md)
- [共通Clientのコンテナ画面観測](common-container-observation.md)
- [Java 1.21.11の移動属性と位置の由来](survival-movement-foundation.md)
- [Java 1.21.11の限定移動制御と実試行の制約](survival-motion-controls.md)
- [検査付きサバイバルAPIと利用側の責務](survival-api.md)
- [取得場面と仮想の移動・編集・照準](survival-hypothetical-scenes.md)
- [採掘中の受信inventory変化の診断](survival-inventory-interruption.md)
- [Headless client API拡張ロードマップ](headless-api-roadmap.md)
- [API契約と所有権](api-contracts.md)
- [状態・イベントの扱い](state-and-events.md)
- [Protocol 736 coverage](protocol-coverage.md)
- [設計と責任境界](architecture.md)

初めて利用する場合は「導入と最初の接続」から読み、操作の一覧は「公開API」、対応可否は「対応機能と制約」を参照してください。

## 開発・検証

- [開発とテスト](development.md)
- [client統合・main移行の差分と検証記録](develop-integration.md)
- [Server teleport後の位置補正調査](teleport-investigation.md)
- 実サーバーの構築（repository版の`test-server/README.md`）
- 耐久試験の結果（repository版の`reports/`）

## 完了済み計画と記録

以下は現行仕様ではなく、実装経緯と検証証跡を保存する文書です。

- [物理ロードマップ](history/physics-roadmap.md)
- [物理実装完了報告](history/physics-completion-report.md)
- [サバイバルロードマップ](history/survival-roadmap.md)
- [サバイバル実装完了報告](history/survival-completion-report.md)

Read-only caller assumptions: [assumed survival scenes](assumed-survival-scenes.md).
