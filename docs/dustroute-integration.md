# DustRoute 向けクライアント導入調査

状態: クローンと専用ブランチを用意。大きな前提作業が判明したため、移植実装は未着手。
調査日: 2026-09-29。

## 作業場所と基準

- リポジトリ: <https://github.com/sugipamo/Voxrig>
- ローカル: `/root/Voxrig`
- remote: `upstream`（上記リポジトリ。push はしていない）
- ブランチ: `codex/dustroute-client`
- 調査した上流コミット: `6434b2cd8d7328d397b34b0151660a9882d844fc`
- 接続先プロジェクト: `/root/DustRoute`、基準 `b1762b9`
- Voxrig は MIT。生成データ・fixture の出典は既存の
  [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) を維持する。

目的は、自分たちで受信・状態反映・観測の因果関係を追跡できるクライアントを持ち、
検証できた範囲でコマンドによる常時照合を不要にすること。
Rust 化だけでサーバー状態との一致が保証されるとは扱わない。

## 既存の土台

Voxrig は接続・packet parser・chunk cache・型付き操作 API を持つ。
world の snapshot は lock 内で取得し、領域別 revision を付けられる。
`ChunkSnapshot` は Arc による共有、更新時の copy-on-write を使う。
`None` は未ロードであり Air ではないという区別もある。
受信とクライアントの状態管理を Voxrig、回路の設計・シミュレーション・採用・診断を
DustRoute に置く責務分担は可能。

根拠: [architecture](architecture.md)、[state-and-events](state-and-events.md)、
`src/client.rs::observe_snapshot` / `block_snapshot`、`src/world.rs::ChunkSnapshot`。

## 導入前に必要な変更

| 項目 | 確認した現状 | DustRoute に必要な対応 |
| --- | --- | --- |
| Minecraft バージョン | Java 1.16.1、protocol 736 固定 | 対象の Java 1.21.11 へ対応。定数変更だけでは接続・decode できない |
| 接続状態とパケット | 1.16.1 の login/play、固定 ID の分岐 | 対象版の login/configuration/play、必須応答、切断・再設定の扱いを整備 |
| chunk と dimension | 1.16.1 の bitmap/16 section/biome 形式 | 対象版の chunk、dimension 高さ、負の Y、palette と unload/再ロードに対応 |
| block state | 1.16.1 の state ID と名称が中心 | 対象版の ID と全 properties の対応を持ち、DustRoute の native state へ欠落なく変換 |
| ピストンによる更新 | Block Action はイベントとして公開 | ピストン通知、通常の状態更新、必要なクライアント処理の関係を対象版で検証 |
| 観測の出所と時刻 | ローカル revision と時刻、broadcast event | 接続世代、受信順、状態反映順、snapshot の境界、欠測を追跡できる観測 API |
| DustRoute 接続 | 現行は Mineflayer 向け bridge と確認証拠を要求 | Rust adapter と観測契約を設計し、操作の受理と配置確認を分離 |

実装上の根拠:

- `src/protocol.rs::PROTOCOL_VERSION`、`src/client.rs::CLIENT_INFO` と接続処理。
- `src/client.rs::read_loop` の `0x0a` は `Event::BlockAction` を emit する。
  その分岐には、移動先や隣接階段の形状を更新する処理はない。
  この事実だけで 1.21.11 の階段問題の原因を確定したとは扱わない。
- `src/world.rs::apply_chunk` / `apply_multi_block_change` は 1.16.1 の形式。
- `src/registry.rs::BlockData` と `block_name_from_state` は、DustRoute が必要とする
  完全な state properties の公開変換 API にはなっていない。
- `docs/state-and-events.md` の revision は接続内・同じ状態領域内で比較するもの。
  サーバーの game tick やサーバー確認の証拠に読み替えない。
- DustRoute の `crates/dustroute-mcp/src/bridge.rs::ConfirmedRegion::validate` と
  `bridge_protocol.rs::PhysicalPlacementMode` は、現在の観測・実行方式に依存する。
  ローカル snapshot に既存の `server_confirmed` を付けて通す変更はしない。

現在の両プロジェクトの隔離試験は offline-mode を使用できる。
Microsoft 認証の追加は初期導入の前提にしない。必要になった時点で別途範囲を決める。

## 推奨する段階と完了条件

1. **1.21.11 の観測用クライアントを成立させる。**
   必須の接続・状態遷移・応答と chunk/block 更新を先に対応する。
   矩形領域の全セル・全 properties と未ロードを区別して取得できること、
   再接続・再設定・dimension 変更で旧 cache を混同しないことを確認する。
   protocol・registry データは版と出典を固定し、現在の 1.16.1 機能をそのまま
   1.21.11 対応と表示しない。

2. **階段問題を受信から snapshot まで追跡する。**
   受信 packet、decode 結果、world revision、snapshot の対応を保存・再生できるようにする。
   隔離環境で既存のピストン・階段 fixture を使い、サーバーの記録と比較する。
   正しい state packet の未反映、Block Action から必要なクライアント処理の不足、
   受信情報だけでは確定できない状態を区別する。
   最終状態だけでなく、遅延・chunk 再ロード・欠測時に古い観測を採用しないことも確認する。
   この段階のコマンド照合や計測 MOD は、移植の比較検証用として使える。

3. **DustRoute を Rust API へ接続する。**
   まず観測 adapter、次にレバー入力・配置・撤去を移す。
   視線・対象プレイヤー・移動・inventory を含む既存 bridge の必要 API を棚卸しし、
   対応した機能だけを公開する。Voxrig に Blueprint や回路シミュレータを持ち込まない。
   観測根拠の違いを型と採用条件に明示し、ローカル予測をサーバー観測に偽装しない。
   参照ドア、フライングマシン、修理・取消しを隔離環境で確認する。

4. **検証できた範囲でコマンド確認と Mineflayer 依存を除去する。**
   新クライアントを使って対象の操作を一巡でき、欠測・切断・外部変更を適切に扱えることが条件。
   受信情報だけでは確定できないケースが残れば報告し、保証を暗黙に弱めない。
   同じコードを使うシミュレータの予測との一致だけでは、独立した実機検証の代わりにしない。

## 今回の停止点

ユーザーの従来の指定「大きなブロック要素があった場合は作業全体を停止して報告」に従い、
**1.16.1 から 1.21.11 へのクライアント移植を大きな前提作業として報告する。**
クローン、専用ブランチ、調査書までを準備し、機能実装には入っていない。

コード変更、Cargo のビルド・テスト、実サーバー起動、ワールド変更、remote push は未実施。
DustRoute 側の依存関係・既存 bridge・コマンド確認も変更していない。
ビルド可否、実接続、性能、コマンド確認を完全に廃止できるかは未検証。

上流への大きな変更を提案する場合は [CONTRIBUTING.md](../CONTRIBUTING.md) の手順も確認する。
今回、外部への issue・PR・メッセージ送信は行っていない。
