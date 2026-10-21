# 共通の採掘（`Survival::dig`）

`client.survival().dig(target, face)`は、手に持っているもので任意のblockを掘る。両版で同じ型。
`dig_estimate(target)`は、送らずに掘る時間だけを返す。

```rust,no_run
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let survival = client.survival();
let estimate = survival.dig_estimate([10, 64, 3]).await?;   // tick数・正しい道具か・速さ
let record = survival.dig([10, 64, 3], BlockFace::Up).await?;
println!("{} ticks, removed: {}", estimate.ticks, record.removed);
# Ok(())
# }
```

## 動き

公式clientと同じ順で送る。

1. START_DESTROY_BLOCKを送る。進み方が1以上なら、ここで壊れる（`ticks`が0）。
2. 1 tickごとの進み方を`ticks`回足して1に届くまで待つ（clientの時計。1 tickを足す）。
3. STOP_DESTROY_BLOCKを送る。serverは自分が数えたtickで70%以上進んでいれば壊す。
4. 受信したblockの状態が変わるのを最大1秒待ち、`removed`と`final_state`を返す。
   `removed` は元の world で loaded air を観測したときだけ true。アンロードや別の non-air block は false。
   壊れなかったことはerrorではない（serverが決める）。1.21.11は`sequences`にSTARTとSTOPの操作番号が入る。

70%（0.7）の判定は低 TPS や START/STOP の処理遅延を吸収する保証ではなく、
server が数える tick が不足すれば、この余裕があってもその場で壊れない。

送信前に、受信したgame modeがsurvivalであること、対象がloadedで目から4.5以内であること、
未解決の操作がないことを検査する。START/STOP の直前にも元の world/mode/選択した hand と採掘時間の入力を照合する。
各 adapter は同じ状態ロックを保持したまま検査から完全なパケット送信まで行う。
対象・目の水位・接地・player capture もそのロック内で採取する。
両版の START/STOP で、元の選択 slot・item の id/名前・data が同じなら、
変更前後の個数がともに正（0 より大きい）である個数だけの増減を許容する。
1.16.1 の通常摩耗は、非負で単調増加する Damage だけの差を許容し、個数の増減とも組み合わせられる。
個数 0、Empty と Item の入れ替わり、Unavailable、他の NBT（エンチャントなど）や
components の変更（1.21.11 の damage を含む）、item や選択の変更は拒否する。
snapshot で区別できない同じ item/data の別 stack への交換は検出できない。
START では採掘見積もり全体の完全一致を要求する。STOP では、硬さ（`hardness`）と
正しい道具か（`harvestable`）が同じで、`progress_per_tick` が初期値以上、`ticks` が初期値以下なら
速くなった見積もりも許容する。両方の見積もりの `speed` と `progress_per_tick` は有限かつ正でなければならない。
f32 を直接比較し、誤差の許容幅は設けない。遅くなった場合や、これらの条件に合わない変化は拒否する。
着地や水から出た場合も、待機時間は最初の見積もりの `(ticks + 1) * 50 ms` のままで短縮せず、
`DigRecord.estimate` も最初の見積もりを保持する。`ticks == 0` の瞬時採掘は従来どおり STOP を送らない。
この許容はローカルな整合性の判断であり、block が壊れる保証ではない。server は自分の tick と状態で決める。
server の接地（`on_ground`）や水中の状態は受理した移動 packet に由来し、こちらより遅れたり異なったりする。
水位や pose の違いによって server 側の採掘速度がこちらより遅くなることもある。
拒否メッセージは変化した前提条件と変更前後の値、START/STOP の phase、STOP 時の START 送信後の経過 ms、player y/vy を示す（診断用のみで比較条件には追加せず、文面は API 契約ではない。vy が得られない場合は n/a）。
単発診断用の `start_mining` と異なり、`dig` は元接続を永久に採掘専用へ隔離せず、
同じ接続で次の操作を呼べる。戻り値は server ACK や除去した actor の証明ではない。
START の後で `dig()` の future を drop した場合、STOP が完全送信済みか瞬時採掘の場合を除き、
接続が所有する task が元の対象へ ABORT を一度だけ試みる。
採掘条件の変化で STOP を拒否した場合も同じ cleanup を使う。元の world/mode が変わった場合は
新しい文脈へ ABORT を送らず、cleanup を `ContextChanged` として記録する。
I/O が不確実な場合は自動再送・再接続せず、同じ world の次の採掘を拒否する。
`dig_cleanup_record()` は取消・拒否後の `Pending` / `Aborted` / `ContextChanged` / `Failed` を
切断後も読める診断で、server の ACK・採掘除去・再実行権限ではない。
`Aborted` はローカルで ABORT の完全な書込を確認できた場合だけを表す。
書込失敗や応答消失で ABORT の配送が不明な場合は `Aborted` ではなく `Failed` を記録し、
`ErrorKind::OperationUnconfirmed` を返す。
ABORT の拒否・書込失敗は `ErrorKind::OperationUnconfirmed` を返し、元の STOP 拒否理由と
cleanup 失敗を診断チェーンへ保持する。呼出側は変更操作を停止して原記録を検査する。
ABORT が完全送信された場合は元の拒否エラーを返す。次の採掘を未解決 cleanup で拒否する場合も
`OperationUnconfirmed` であり、入力検査だけの `State` と区別できる。
同じ接続の運用採掘は一度に一つで、active な運用採掘の間は診断 mining の新規開始も拒否する。
STOP が完全送信済みか START だけで瞬時採掘が完了した後には、取消による ABORT を追加しない。
1.16.1 では送信済み STOP によって server が遅延破壊を予約すると、ABORT でもその予約は取り消せない。
1 秒の観測 timeout の後も含め、block が後から壊れることがある。

## Per-dig cancellation handle (begin_dig / DigOperation)

**Dropping all `DigOperation` handles, or dropping/timing out any `cancel()` or
`finished()` waiter, does NOT cancel the dig. It keeps digging.** Cancellation
is explicit through `request_cancel()` (or the eager `cancel()` method).
The existing lazy `dig()` API retains its cancel-on-waiter-drop behavior.

```rust,no_run
use voxrig::client::prelude::*;
# async fn run(client: &Client) -> Result<()> {
let operation = client.survival().begin_dig([10, 64, 3], BlockFace::Up)?;
let id = operation.id();
// On the caller's stop signal, before awaiting anything:
operation.request_cancel();
// The caller applies its own remaining deadline to this passive waiter.
let completion = operation.finished().await;
assert_eq!(completion.id(), id);
match completion.cancel_outcome() {
    DigCancelOutcome::StartSuppressed | DigCancelOutcome::Aborted => { /* inspect result + owner_exit too */ }
    _ => { /* retain evidence; do not infer interruption or retry */ }
}
# Ok(())
# }
```

`begin_dig(&self, target: [i32; 3], face: BlockFace) -> Result<DigOperation>` is
synchronous: it claims the single dig slot, allocates the identity and registers
the connection-owned task without waiting for readiness, state locks or I/O.
An error guarantees that this invocation can never send START. The owner waits
behind a registration latch until setup succeeds. A current Tokio runtime is
required; absence of one returns `ErrorKind::State`, without panicking. START
can race a successful return. A second `begin_dig` or a polled `dig()` is rejected
immediately while the slot is held, rather than queued.

`DigOperation` is cloneable, Send + Sync, and retains only per-operation state,
not the live Client. `id()` returns an opaque process-local transport identity
plus a monotonically allocated ordinal internally. IDs are not reused across
world resets; exhaustion is refused without wrapping. There is no ID lookup,
serialization, persistence, reconstruction or ID-based cancellation authority.
An old handle can never cancel its successor, even at the same target/session.

- `request_cancel(&self)` synchronously latches and wakes, without adapter/writer
  locks or I/O. Repeated/concurrent calls request the same single owned cleanup.
- `snapshot(&self) -> DigSnapshot` reads provisional phase, original session,
  cancellation and independent START/STOP/ABORT evidence. Its `finished()` is
  absent until actual join; provisional `NotSent` may still become committed.
- `cancel(&self) -> impl Future<Output = Arc<DigCompletion>> + Send + '_` calls
  `request_cancel()` **before returning its future**, even if never polled.
- `finished(&self) -> impl Future<Output = Arc<DigCompletion>> + Send + '_` is
  passive. Every reader, including cancellation after natural completion, gets
  the same immutable `Arc` (`Arc::ptr_eq`), without changing the result.

The approved race boundary is **STOP committed**. One per-dig state machine
orders cancellation against START and STOP. On 1.16.1 capacity is reserved first,
then the adapter resnapshots/validates and commits immediately before enqueue,
without an await between commit and enqueue. On 1.21.11 final validation and
commit happen while holding writer serialization, immediately before the write.
Passing validation or constructing a future is not commitment. Cancellation
before START commitment suppresses START. After timed START Written, cancellation
before STOP commitment suppresses STOP and requests one eligible original-context
ABORT. Cancellation after STOP commitment lets that write finish; STOP Written
means `AlreadyStopped`, even if it finishes after the request. No cancellation
ABORT follows a committed uncertain/written STOP. A provably refused STOP remains
`NotSent` and shares the same single cleanup path with cancellation. Faster STOP
estimates accepted under the conditions above use exactly the same boundary.
Normal target observation is preserved after STOP or instant START, including
when cancellation arrives late.

All new structs have read-only accessors, and all new enums are non-exhaustive.
`DigCompletion` separates packet disposition, original `DigResult`, and actual
`DigOwnerExit`; callers must inspect all three. `cleanup_error_kind()` and
`cleanup_message()` retain a cleanup failure separately from the original cause.
All diagnostic messages/reasons are unstable and may change; branch on typed
variants, categories and evidence, never on wording.

| Evidence / outcome | Meaning and invariant |
| --- | --- |
| `NotSent` | In terminal completion, sealed: this write cannot later be dispatched. |
| `Pending` | Committed but unresolved; snapshots only. Every remaining Pending becomes Unknown at join. |
| `Written { interaction_sequence }` | Complete local write; 1.21.11 also requires successful flush, 1.16.1 requires the actor's explicit Dispatched outcome. Never a server ACK. |
| `Unknown` | Possibly partial or complete write, DeliveryUnknown or lost reply without independently proved evidence. Never silently changed to Written or NotSent. |
| `StartSuppressed` | START NotSent. Result may still be an independent admission failure. |
| `Aborted` | ABORT Written and STOP definitely NotSent. Exactly one owned ABORT attempt; no rollback claim. |
| `AlreadyStopped` | STOP Written; cancellation cannot retract it. |
| `InstantBreak` | START Written with initial ticks == 0. No STOP or cancellation ABORT. |
| `Completed` | Target changed before STOP; no assertion about who changed it. |
| `ContextChanged` | Original world/mode/session changed. No ABORT into a replacement context. |
| `Disconnected` | Transport ended or teardown cancelled the owner; retain all previous evidence. |
| `Failed { kind, message }` | Known failure without a stronger packet disposition. |
| `Unknown { reason }` | Unresolved write or abnormal owner evidence without a stronger disposition. |

Classification precedence is proved STOP, instant START, eligible ABORT, proved
START suppression, context change, disconnect, uncertainty, then known failure
or target-change completion. Evidence is never erased by this precedence.
`DigResult::Finished(DigRecord)` and cooperative `Cancelled` require
`DigOwnerExit::Returned`. A cancellation request alone never changes an independent
failure into `Cancelled`. Panic and forced cancellation produce `Failed` results
with `Panicked` or `Cancelled { connection_teardown }` owner exit; they can coexist
with independently proved packet dispositions. `Aborted` plus a failed result
can therefore mean STOP was refused and cleanup was written, rather than success.
Confirmed cooperative interruption requires Returned + Cancelled + StartSuppressed
or eligible Aborted, with consistent known evidence.

**`finished()` means only that the dig owner task was actually joined**, after
its guards and finite-work bookkeeping finalized. Publication comes from the
connection task registry on return, panic, and teardown cancellation. It does not
prove connection shutdown or retirement of unresolved queued actor work; use
`client.wait_shutdown()` separately. If supervision/runtime is lost, no receipt
is fabricated and a waiter may remain pending. The caller owns deadlines, and
may use its existing revocation policy at expiry. No SDK timeout, grace period,
retry or reconnect is added; dropping a timed-out waiter does not retract cancel.

**1.16.1 caveat:** a STOP below 0.7 progress can schedule delayed server destruction;
ABORT does not clear that schedule. Even ABORT Written does not prove an unchanged
block or server stillness. Instant START can already have broken the block.
Sequences and acknowledgements do not supply rollback, actor identity, or authority
to replay an uncertain mutation. Diagnostic mining and native timed dig helpers
retain their existing contracts.

## 掘る時間の計算

公式の`BlockStateBase.getDestroyProgress`と`Player.getDestroySpeed`をそのまま計算する（f32）。

- 進み方 = 速さ ÷ 硬さ ÷ （正しい道具なら30、そうでなければ100）。硬さが負のblock（岩盤など）は拒否する。
- 硬さ・正しい道具が要るか・各itemの既定の速さと正しい道具かどうかは、両版の公式server jarで全block state
  （1.16.1は17,104、1.21.11は29,671）について公式のgetterを呼んで書き出した表による
  （`scripts/export_dig_profiles.py`、`data/client_api/dig_profiles-{版}.json.gz`）。
- 速さに掛かるもの:
  - 効率強化: 1.16.1は手に持ったitemのNBTの`Enchantments`（速さが1より大きいとき、レベル² + 1を足す）、
    1.21.11は受信した`minecraft:mining_efficiency`属性。
  - 採掘速度上昇・コンジットパワー（大きい方）: ×(1 + 0.2 × (レベル + 1))。採掘速度低下: ×0.3・0.09・0.0027・0.00081。
  - 1.21.11の`minecraft:block_break_speed`属性。
  - 目が水中: 1.16.1は兜に水中採掘がなければ÷5、1.21.11は`minecraft:submerged_mining_speed`属性（既定0.2）を掛ける。
    目が水中かどうかは、受信した目の高さのblockの水位で決める。
  - 地面にいない: ÷5。継続操作の実行中はそのframe、それ以外は各版の自分の状態の接地を使う。
- 属性と効果は受信した値（[自分の状態](common-player-facts.md)）。受信していない属性はclientの既定値を使う。
- 手に持ったitemの「速さ」は既定のitemの値で、itemごとに付け替えた`minecraft:tool`部品（1.21.11）などは反映しない。

## 実サーバーでの確認（2026-10-08）

公式`server.jar`（1.16.1・1.21.11）をoffline-mode・peacefulでlocalhostに起動し、`examples/dig_probe.rs`で
consoleから置いたblockを掘った。両版とも、すべてserverが壊した（`removed`）。

| block・道具 | tick | 1.16.1の所要 | 1.21.11の所要 |
| --- | --- | --- | --- |
| 土・素手 | 15 | 1.00 s | 1.03 s |
| 石・素手（正しい道具ではない） | 151 | 7.62 s | 7.65 s |
| 石・木のつるはし | 23 | 1.22 s | 1.24 s |
| 石・ダイヤのつるはし | 6 | 0.38 s | 0.40 s |
| 石・効率強化Vのダイヤのつるはし | 2 | 0.17 s | 0.19 s |
| 黒曜石・ダイヤのつるはし | 188 | 9.47 s | 9.50 s |
| 草（硬さ0） | 0（STARTで壊れる） | 0.02 s | 0.04 s |
| 石・木のつるはし・採掘速度上昇II | 17 | 0.93 s | 0.95 s |
| 鉄鉱石・木のつるはし（正しい道具ではない） | 151 | 7.63 s | 7.65 s |
