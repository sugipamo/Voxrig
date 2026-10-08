# 共通chat・command

`Client::send_chat` / `Client::send_command` / `Client::chat_after`は1.16.1・1.21.11の両方で使える。
ゲームモードに依存しないため、`survival()` / `creative()`ではなく`Client`に置く。

## 送信

- `send_chat(message)`: 1～256文字。制御文字・`§`・先頭の`/`は送信前に拒否する。
  1.21.11では署名なしのchatとして送る（offline-mode、`enforce-secure-profile=false`のサーバーが前提）。
- `send_command(command)`: 先頭の`/`はあってもなくてもよい。1.21.11は`CHAT_COMMAND`、1.16.1は`/`付きのchatで送る。
- 戻り値の`DispatchReceipt`は送信の完了だけを示す。サーバーが受理・実行したことは示さない。
  権限不足のcommandは送信に成功し、結果はsystem chatで返る。

## 受信履歴

`chat_after(cursor)`は受信連番が`cursor`より後のメッセージを古い順に返す。
最初は`0`を渡し、以後は返り値の`receive_sequence`を次の`cursor`にする。

- 保持は最新256件・合計4MiBまで。`cursor`より前が捨てられていた場合はエラーにし、欠けた履歴を返さない。
- 本文は受信した形式（1.16.1はJSON、1.21.11はNBT）のまま`UiText`で返す。描画・翻訳は利用側の責務。
- 1.21.11のプレイヤーchatは、サーバーが装飾した`unsigned_content`があればそれを、なければ本文の文字列（`ChatText::Plain`）を返す。
- 1.21.11の`PLAYER_CHAT` / `PROFILELESS_CHAT`の内容を解読できない場合（独自のinline chat typeなど）は、
  `ChatText::Undecoded`として記録し、接続は切らない。
- 送信順と受信順は一致しない。1.21.11ではサーバーがchatを非同期に処理するため、後から送ったcommandの結果が先に届くことがある。

| `ChatKind` | 1.16.1 | 1.21.11 |
| --- | --- | --- |
| `Player` | `CHAT` position 0 | `PLAYER_CHAT` |
| `Profileless` | — | `PROFILELESS_CHAT`（consoleの`say`、署名なしの`/me`等） |
| `System` | `CHAT` position 1 | `SYSTEM_CHAT` overlay=false |
| `ActionBar` | `CHAT` position 2 | `SYSTEM_CHAT` overlay=true |

1.21.11の`title ... actionbar`は`SET_ACTION_BAR_TEXT`で届くため、chat履歴ではなく
[title等の観測](common-ui-display.md)に入る。

## 実サーバーでの確認（2026-10-07）

公式`server.jar`（1.21.11 SHA-1 `64bb6d76…`、1.16.1 SHA-1 `a412fd69…`）をoffline-modeでlocalhostに起動し、
`examples/chat_probe.rs`で確認した。

| 版 | chat送信 | `/me`送信 | 受信して解読できた種類 |
| --- | --- | --- | --- |
| 1.16.1 | サーバーログ`<ChatProbe> hello from voxrig` | `* ChatProbe waves from voxrig` | Player（chat・emote）、System |
| 1.21.11 | `[Not Secure] <ChatProbe> hello from voxrig` | `[Not Secure] * ChatProbe waves from voxrig` | Player（`Plain`本文）、Profileless、System |

1.21.11の`PLAYER_CHAT`と`PROFILELESS_CHAT`の実際のpayloadは`src/client/chat.rs`のtestに保存し、
過不足なく解読できることを検査している。
