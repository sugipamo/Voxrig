# API契約と所有権

この文書は、外部controllerが`voxrig`の戻り値と完了をどう解釈するかを定義します。関数シグネチャの正本はrustdoc、用途別一覧は[公開API](api.md)です。

## Stableとunstable

crate root、`prelude`、`Bot`、`BotManager`から直接利用するAPIを通常の公開面とします。`unstable` moduleは、protocolに近く誤用時にserver補正や切断を招く操作です。`0.x` release間で互換性を保証しません。

現在のunstable操作は、client物理を通さない直接相対移動、acknowledgementを待たないraw digging action、client-authoritativeなvehicle pose送信です。

```rust
bot.unstable().move_relative(0.1, 0.0).await?;
```

通常移動には`set_control()`、block破壊には`dig_block()`を使用してください。

外部controller向けには、`OperationContext`を必須とする
`Bot::dispatch_operation()`と`Bot::dispatch_cleanup()`を用意しています。
前者は呼出側が選んだ単一の低レベル操作、後者はdisconnecting中にも有限に
許可されるdig cancel、use stop、control clear、window closeだけを受理します。
`DispatchOutcome::Dispatched`はwriterの事実であり、protocolの
`Acknowledged`やfresh observation／semantic successを意味しません。
write境界が不明な場合は`DeliveryUnknown`として扱います。`ControlClear`だけは
packetを持たないため、`CleanupDispatchOutcome::AppliedLocally`で表します。

`AcknowledgedOperation`（WindowClick、DigFinish）は、接続actorが
connection-localなtransaction identity、pending queue、confirmation order、
ack deadlineを一元所有するtyped APIです。`ProtocolTransaction::wait()`の
`Acknowledged`／`Rejected`／`DeliveryUnknown`はprotocol事実であり、fresh
observationやsemantic successではありません。Futureをdropしても、既に
writeされたpacketやactorのpending transactionは取り消されません。未確認の
pending transactionは接続actor内で最大1024件に制限され、上限時は新しい
acknowledged dispatchを`TransactionInProgress`としてwrite前に拒否します。
Digging Ackはpositionと要求status（Finished）で相関し、block-state IDは
結果値として扱います。

既存の`click_slot*`互換APIは同じactor queueを使うadapterです。
`InventoryState::pending_clicks`は予測状態のrollback用metadataだけを保持し、
transaction identity、ack期限、確認結果の正本ではありません。Window
ConfirmationとDigging Ackはpacket applyからactorへroutingされ、既存eventの
発火とtyped protocol outcomeを分離します。未対応のack protocolは二重queueを
作らず、明示的な後続範囲として残します。

## 操作完了の段階

`Result::Ok`は操作ごとに次のいずれかを意味します。

| 段階 | 意味 | 代表API |
| --- | --- | --- |
| local | client内へ要求を保存 | `set_control`、`jump` |
| dispatched | packetのwrite成功 | `look`、`place_block`、`use_item`、`attack`、chat |
| acknowledged | 対応するserver応答を確認 | `dig_block`、`click_slot_and_wait` |
| observed | 状態snapshotで結果を確認 | 利用側が`block`、`inventory`、eventで確認 |

`raycast_blocks()`、`digging_info()`、`placement_info()`はpacketを送信しない事前観測です。結果は呼出時のlocal world cacheとplayer状態に基づきます。server permission、plugin規則、同時更新は含みません。

`dispatched`はゲーム内結果の成功を保証しません。例えば`place_block()`が成功しても、距離、権限、衝突などを理由にserverが設置を拒否できます。結果が重要な処理は、関連eventまたはsnapshotで確認してください。

Futureをdropしても、既にdispatchしたpacketは取り消されません。特に`use_item_for()`を途中でcancelした場合、利用側が`release_item()`を送る必要があります。

## 継続入力と単発要求

`ControlState`は置換されるまで継続します。`set_control()`の完了はlocal state更新だけを意味し、内部20 Hz loopが後続tickで反映します。

`jump()`は次のphysics tickに対するedge-triggered要求です。水中浮上やツタ上昇のようなhold入力には`ControlState::jump = true`を使います。

## Eventとsnapshot

Eventは変更通知で、完全なevent sourcing logではありません。broadcast receiverが遅れると`Lagged`になり得ます。その場合はgetterまたは`*_snapshot()`から現在状態を再取得します。

`Snapshot<T>`は次を持つowned valueです。

- `revision`: 状態領域ごとの単調増加番号
- `updated_at`: 接続後、その領域が最後に変更された時刻
- `captured_at`: snapshotを作成した時刻
- `value`: owned状態

revisionは同じBotの同じ領域だけで比較できます。異なるBot、playerとinventoryなど異なる領域、再接続前後では比較できません。

```rust
let snapshot = bot.inventory_snapshot().await;
if snapshot.revision != previous_revision {
    send_to_planner(&snapshot.value);
}
```

## 所有とmemory cost

`Bot::clone()`は同じ内部状態を共有する軽量handleです。一方、getterとsnapshotは内部lockを保持させないため、現在はowned cloneを返します。

| API | 所有・確保 |
| --- | --- |
| `player`、`motion`、`control` | 小さいowned copy |
| `survival_state` | mapを含むowned clone |
| `inventory` | slot、NBT、click prediction metadataを含むowned clone |
| `player_list` | list全体のowned clone |
| `observe_entities` | 対象entityのowned clone |
| `observe` | 毎回新しい`Vec`を確保 |
| `*_snapshot` | 対応getterと同じowned dataにmetadataを付加 |

revisionは不要な再転送を避けるために使えます。通常のgetterはownedですが、`ChunkSnapshot`のsection・light・NBT bufferとmap colorは`Arc` backedです。同じ`BotManager`のBotは内容が同一のchunk sectionを共有し、block更新時だけcopy-on-writeします。chunkのload/unload所有権そのものはBotごとに分離されます。

## Compatibility確認

`Bot::client_info()`、`Bot::protocol_info()`、`Bot::capabilities()`でbuildの能力を確認できます。接続中の宛先は`bot.server_info()`です。

現行buildはJava Edition 1.16.1、protocol 736、offline-mode専用です。online-mode認証、音声再生、他protocol versionは未対応です。

接続・login・play packet・ready待機のtimeoutと、接続ごとのchunk・entity・map・総cache record、CustomPayload、event queueの上限は`ConnectionOptions`で変更できます。上限超過は接続errorとして終了します。自動再接続とbackoffは外部controllerの方針です。

多くのfallible APIは`voxrig::Result<T>`を返します。`craft_once_checked`、staged quiesce、`wait_shutdown`等は専用のerror型を返します。再試行や入力修正の判断にはerror文字列ではなく`Error::kind()`を使用します。`ErrorKind`は将来追加され得るため、matchにはfallback branchが必要です。

## Staged quiesce and source fencing (#131 PR 4)

### retire／退役の用語

Voxrigの接続ライフサイクルは`begin_quiesce`、`admitted_drain`／`seal_for_fence`の
drain段階、`StopPermit`／`StopReceipt`、`revoke_connection`、
`wait_shutdown`／`ShutdownError`で表します。汎用の「retire／退役」は定義しません。

- Golemkitの「退役」は旧Bodyを使用不能にすることです。Voxrigでは接続・世代の
  revokeと、古い証拠で新しい操作を許可しないことに対応し、shutdown完了とは別です。
- evoltoの「退役」はshutdown確認完了後に許可される運用判断です（案2の確認付き
  detach／`ShutdownReceipt`契約）。Voxrig側の対応する事実は、元の接続での
  `Client::wait_shutdown()`の`Ok(())`です。これをdetachに結び付けた受領証にするのは
  Golemkitの責務で、Voxrigは`ShutdownReceipt`を定義しません。
  shutdown確認だけでevoltoの他の運用条件まで満たすわけではありません。
- Voxrigの[採掘retirement](survival-mining-retirement.md)、
  [ack identityのretirement](r3-primitive-dispatch.md)、entityやownerの退役は
  各領域固有の用語で、上記2つの意味とは別です。ownerの退役はそのownerのwrite権限の
  解放を指し、接続のshutdown完了を意味しません。

実装側の定義は[用語注記](internal/drain-ledger.md#terminology-retire-and-retirement)を参照してください。

### 段階と保証

`Client::generation_key()` observes one immutable process-local transport key.
World resets do not replace it. Retain the original source `Client` and key
through handoff; neither a key nor a serialized receipt recreates authority.
Native users enter through the common `Client` (`NativeClient::client()` for
1.21.11). Native clones share exactly the same admission gates and ledger.

`begin_quiesce(key)` is synchronous, irreversible and succeeds once, at Ready.
It closes normal mutation admission, fixes the first root cut and requests
ordinary-producer park. Existing Result-returning mutation, native and raw send
operations on that connection reject with the existing `ErrorKind::State`.
Existing typed native dispatch APIs retain their existing typed admission-error
representation. The three void-returning 1.16.1 native methods `set_control`,
`clear_control`, and `suspend_movement_for` become **no-ops** after begin: no state
change, queueing or write. `ConnectionStatus::Ready` continues to describe the
transport, and alone does not prove mutation admission. Clients that never call
begin retain their existing protocol behavior, outcomes and cancellation rules.

Retain `QuiesceHandle`; it cannot be cloned. Dropping it never resumes, selects a
stop, revokes the source, or cancels accepted work. Lost orchestration authority
requires explicitly fencing that same source; a second begin cannot recover it.
There is no resume and no automatic stop-target selection.

The two stages are deliberately separate:

1. `q.admitted_drain()` observes the fixed first cut and producer park ACKs. Its
   `ProducersParked` receipt covers accepted finite work and all descendants,
   pending protocol transactions, exact barriers, pending swaps and retained
   continuations. Ordinary control and passive/native physics cannot sample
   again. A ground jump accepted before begin but not yet sampled remains finite
   debt: it is evaluated/sent once on the next tick before park is confirmed.
   Independent protocol maintenance may still write after this first receipt.
2. Inspect `q.stop_targets().await`, match **original** world/mode/owner IDs, and
   explicitly choose targets. `q.stop_permit()` borrows `&mut q`, allowing one
   stop holder. `neutralize(target).await` admits at first poll, owns accepted
   cleanup independently of its waiter, and joins/caches repeated requests for
   the same binding without resending. Dropping an unused permit sends nothing.
   For synchronous admission under a caller gate, use
   `permit.enqueue_neutralize(target)?`, inspect `completion.admission()`, release
   the gate, then `completion.wait().await`. The owned completion does not borrow
   the permit and can be dropped before final seal without cancelling work.
3. `q.seal_for_fence()` closes stop and independent maintenance admission and
   fixes the final cut. Its `SealedForFence` receipt waits for accepted stops and
   dependencies and certifies **no later Voxrig application write** on the source,
   including teleport confirm/position, keepalive and configuration responses.
   Accepted response batches remain counted across every frame/flush. Receivers
   continue processing dependency receipts; new unrelated responses are suppressed
   before obtaining write authority. Repeat seal calls observe the same cut/cache.

`enqueue_neutralize` returns `NewlyAdmitted` only after a new ledger root and
connection-task registry submission are complete. `JoinedExisting` observes an
existing pending/cached result with no additional root or worker. A registry
failure after root insertion returns `StopError::AdmissionInterrupted { target,
drain }`: work may have been admitted/started, so this is never a no-work refusal.
The failure is sticky and retries return the same error (never an `Ok` join
without certified registry submission). Other enqueue
errors create no new work in that invocation; wrong/stale targets still fail the
unissued drain. Worker/reset/revoke errors after `Ok` are completion errors.
Historical cached receipts remain historical after revocation.

The synchronous lock order is stop-book -> ledger state (released) -> task
registry; no ledger-state guard spans submission or rejected-worker destruction.
These ordinary mutexes may block the caller's thread. Enqueue does not await,
block on async locks, refresh native state or perform I/O. Caller-held gates must
respect this order and must not re-enter Voxrig in a conflicting way. Release the
gate before waiting. Workers retain native original-context checks under their
existing state/writer locks and may start before enqueue returns. The connection's
captured Tokio runtime (or current runtime if none was captured) must remain
running; absence of either at submission returns `AdmissionInterrupted`.
Poisoned stop-book/ledger state rejects admission with `Drain(Internal)` unless
an earlier sticky failure already determines the drain error; registry poison
closes submission and returns `AdmissionInterrupted` after root insertion, with
shutdown reporting `Internal`.

The SDK enforces no caller deadline. The caller's admission hook must check it;
time can cross that deadline during mutex contention. A timeout around
`completion.wait()` only cancels observation. Accepted work and drain debt survive.
`neutralize` remains a lazy first-poll wrapper over enqueue and wait, including
no target validation, sticky failure, or work when dropped before its first poll.

Common and legacy native movement have separate original-owner bindings. A
native clear cannot remove a common binding; an ended common owner is retired.
When common control owns wire posture, neutralizing the dormant native target
clears only its local inputs. Neutralizing the common target releases that wire
posture, including inherited sprint/sneak. Callers select each target explicitly.
Mount transitions invalidate movement targets even when the old relation returns.

Movement neutralization releases original held input/sprint/sneak, without
landing, disabling flight, predicting a new position or dismounting. Vehicle
neutralization releases input and paddles on the original mount, without claiming
that the vehicle has stopped. Item-use neutralization is allowed **even when it
has gameplay effects**, such as firing a bow. Callers own that selection policy.
Digging neutralization sends ABORT, never FINISH. An ABORT intentionally submitted
through `StopPermit` is **clean completion only after its cleanup is confirmed**:
`StopReceipt` is Ok and drain may succeed. Cleanup timeout is sticky
`TransactionUnconfirmed`; unknown delivery is sticky `UncertainDispatch`. Other
aborts, expiry, lost owners and dropped caller-owned native helper futures remain
permanent failures. No automatic FINISH/ABORT/release is added for those helpers.
Stale/unsupported/refused selected stops send no replacement-owner write and
latch required-release failure. Temporarily restored mode/world/hand/mount state
does not restore an old binding.

`DrainHandle: Clone + Send + Sync` is an observer, not a Future. Use `.wait().await`
for a receipt or `.is_drained()` for a cheap synchronous check: `Ok(false)` pending,
`Ok(true)` issued/cached, `Err` failed. Polling requires no runtime, await, I/O,
record scan or work progression; it takes short synchronous locks and queries the
live-root prefix. Independent waits share the stage cache and have independent
wake registration. Dropping/timing out a wait cancels that observer only.
There is no `try_receipt` API; use `wait` to obtain the same receipt after a true
poll. Receipt issuance and revoke share an ordering gate: revoke first fails an
unissued drain, issue first preserves historical success. EOF/closure is never
successful drain. The first terminal failure remains sticky for every unissued
stage, even after later ACKs or idle state.

`written_through()` is W: the contiguous prefix of socket **write plus flush**
completion from handshake onward. It is neither a maximum ordinal that skips
failures, a server ACK, peer read, server acceptance nor physical stillness.
Receipt completion additionally requires all covered local cleanup. The isolated
internal completion point is `CompletionPoint::SocketFlush`.

All new types are re-exported from `voxrig::client`. Only `QuiesceHandle`,
`DrainHandle`, `StopPermit`, `DrainError`, `DrainErrorKind`, `StopError`,
`StopTarget`, and `StopTargetKind` are added to the common/root prelude. There
are no native-only bridges, new public traits or new dependencies. PR 5 adds the two precise transaction
error kinds described below and exports ShutdownError/ShutdownErrorKind from
voxrig::client only. Follow final drain with explicit source revocation and
`source.wait_shutdown().await`; disconnect alone is not the completion observer.

Golemkit must expose both drain stages and retain their scopes. An illustrative
Golemkit-side adapter (these types and constructors are **not in this crate**):

```text
fn backend_poll(drain: voxrig::client::DrainHandle)
    -> impl Fn() -> Result<bool, BackendError> + Send + Sync
{
    move || drain.is_drained().map_err(|error| match error.kind() {
        DrainErrorKind::TransactionExpired => BackendError::Timeout(/* retain cause */),
        DrainErrorKind::TransactionOutcomeUnknown
        | DrainErrorKind::TransactionUnconfirmed
        | DrainErrorKind::UncertainDispatch => BackendError::OperationUnconfirmed(/* retain cause */),
        _ => map_other_drain_error(error),
    })
}
// BackendDrain/BackendError names are illustrative: consumer definitions
// could not be fetched with the available credentials (GitHub returned 404).
// Pass this closure to the actual Golemkit BackendDrain constructor.
```

Do not translate a first-stage true into final source completion, infer success
from zero counters, or retry uncertain mutations. Error paths still fence and
await source shutdown; an outer orchestration timeout cannot synthesize a receipt.

Jump admission and queue publication hold the producer's state/coherent lock.
A request still waiting for that lock when begin closes admission has not yet
been accepted and returns State. The producer cannot acknowledge park between
acceptance and publication of a queued jump. This ordering is tested in both
adapters, including the legacy native jump path.


Before `begin_quiesce`, serialize source operations, settle inventory work,
explicitly close the original containers and end item use.
Keep this serialization through begin. New ordinary closes are rejected after
begin; no automatic close/release policy is provided. Complete dispatch and cursor
return receipts do not imply a server close echo or returned crafting inputs.

Run revoke, application source-task cancellation/join and wait_shutdown under a
surviving application supervisor even on drain failure, timeout or cancellation.
Keep the primary stage result and shutdown result separately. Shutdown observation
is passive, targets the original Client generation, and is safe to cancel; all
concurrent/later waiters see the same cached result. Successful local shutdown
certifies released transport halves and connection task exit while clones survive,
not game success or physical stillness. A failed drain stays failed after shutdown.
See [the implementation contract](internal/drain-ledger.md#pr-5-shutdown-completion-and-golemkit-integration)
for scope, error evidence, C1 preflight and the complete handoff sequence.
