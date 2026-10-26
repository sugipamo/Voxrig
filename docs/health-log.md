# Received health history (Golemkit #406)

Both Java 1.16.1 and Java 1.21.11 support the same additive common API,
with or without `native`. Import from `voxrig::client::health`,
`voxrig::client`, or the common prelude. Existing events and signatures are unchanged.

`Client::health_changes_after(after_sequence).await` reads a bounded, complete
suffix. Start with 0, then pass `HealthLog.sequence` to the next read. Each
`HealthChange` has a consecutive health/lifecycle `sequence`, the original
packet's `receive_sequence` (None for local/transport closure), and `event`:

- `Vitals(ReceivedHealth { session, receive_sequence, health })`: exactly one per
  applied `update_health`, including repeated equal values. Health contains raw
  HP, food, and saturation. No entity metadata, prediction, polling, or submitted
  value produces a Vitals record.
- `WorldChanged(SessionStamp)`: join, replacement login, respawn/dimension change,
  and modern reconfiguration. Retained records keep their original world/session.
- `Disconnected`: terminal, including reader failure, revocation, and dropping the
  last client handle. This means this receipt source has ended; it is not proof
  of a clean transport logout. Later records are never appended.

`MAX_RETAINED_HEALTH_CHANGES = 4096` bounds the combined health/lifecycle log,
independently of all gameplay queues. A read behind the retained window returns
`ErrorKind::State` with the number of missing health records; a future health
cursor returns `InvalidInput`. The entire retained suffix remains readable after
closure. There is no batch read limit or implicit capture acknowledgement. A
caller may process fewer records and retain its last processed sequence without
losing the remainder. Packet receive ordinals and health cursors are different
axes: never substitute one for the other.

## Atomic baseline and push subscription

`Client::subscribe_health()?` returns an independent cursor positioned at the
producer tail. `subscription.baseline()` is the latest received HP in the current
world **at that same cut**. It is None before first HP, immediately after a world
boundary, or after closure. Baseline retrieval and the tail cut use one log lock;
every later applied HP receipt remains readable. Do not separately read
`player_state`/`received_player_snapshot` then subscribe: a packet may arrive
between those two calls. The subscription is the combined baseline-plus-tail
operation, with no observation-counter inference.

The version's ordered packet reducer publishes to this log before making the
corresponding latest received cache/snapshot visible. Existing common state reads
also hold the version's application gate. They cannot observe a received baseline
ahead of publication. A subscription can observe the log publication while the
reducer finishes updating compatibility caches; its own baseline always comes
from the log, so it is consistent with its tail. Latest state is not a substitute
for intermediate history.

`try_recv()` reads one record in O(1). `recv().await` waits using Notify, registering
before checking its cursor; no gameplay broadcast forwarding, periodic polling,
or background adapter task is involved. Waiting is cancellation safe.
`HealthRecvError` is `Empty`, `Lagged(n)`, or `Closed`:

- Empty leaves the cursor unchanged and means no new record yet, not lost history.
- Lagged reports exactly the unread health/lifecycle records actually evicted.
  It advances that reader to the retained head so it can inspect later evidence,
  but consumers requiring completeness must terminate their guarded task. A world
  boundary never silently clears a preceding gap.
- The terminal Disconnected record is delivered before Closed. A subscriber
  created after closure receives Closed immediately. Slow readers first observe
  any real lag, then the retained suffix including the terminal record.

`resubscribe()` makes a **new** baseline/tail cut and leaves the original reader
unchanged. Reading, dropping, or resubscribing one subscriber never acknowledges
another. Explicit resubscription discards pre-cut evidence for that new reader;
it must not be used to conceal a task's gap. Use a fresh subscription for a new
connection; compare the full SessionStamp, not just world_generation or sequence.

NaN (including original payload bits), infinity, and negative HP are retained.
`ReceivedHealth::is_valid()` checks finite, nonnegative HP; zero is valid death.
It imposes no HP upper bound and does not validate food/saturation. Check it before
computing minima: ordinary float comparisons can hide NaN. JSON serialization of
nonfinite floats is null, so use the Rust values or explicit bit encoding when
persisting invalid evidence. No invalid numeric health payload is silently omitted.

## Minimal Golemkit admission and minimum

This common-API example compiles without `native`. The baseline and cursor are
established at task admission. Keep the subscription for the full task, even
while a dig runs for several seconds. An Empty read is the end of the currently
available suffix, not a completeness failure.

```rust,no_run
use voxrig::{Client, client::{HealthEvent, HealthRecvError}};

fn begin_guard(client: &Client) -> anyhow::Result<(voxrig::client::HealthSubscription, f32)> {
    let subscription = client.subscribe_health()?;
    let baseline = subscription.baseline()
        .filter(|v| v.is_valid())
        .ok_or_else(|| anyhow::anyhow!("HealthUnavailable"))?;
    Ok((subscription, baseline.health.health))
}

async fn guard_dig(client: &Client, floor: f32) -> anyhow::Result<()> {
    let (mut health, mut minimum) = begin_guard(client)?;
    if minimum < floor { anyhow::bail!("HP floor"); }
    // Admit/run the guarded dig using the caller's existing Body scope.
    // Keep `health` across all awaits; drain at capture/admission boundaries.
    loop {
        match health.try_recv() {
            Ok(HealthEvent::Vitals(value)) if value.is_valid() => {
                minimum = minimum.min(value.health.health);
                if minimum < floor { anyhow::bail!("HP floor"); }
            }
            Err(HealthRecvError::Empty) => break,
            Ok(HealthEvent::Vitals(_))
            | Ok(HealthEvent::WorldChanged(_))
            | Ok(HealthEvent::Disconnected)
            | Err(HealthRecvError::Lagged(_))
            | Err(HealthRecvError::Closed) => anyhow::bail!("HealthUnavailable"),
        }
    }
    // Return/retain the scoped receipt and minimum in the consumer's real task.
    Ok(())
}
```

Golemkit still owns its task scope and capture receipt policy. A rejected capture
must retain the same task cursor/evidence. An accepted, scope-validated capture
may acknowledge only its own cut; retain any later records. Use the cursor API if
you need rereads before acceptance (record each entry's sequence; advance your
acknowledged cursor only after acceptance). Subscription reads consume that
subscriber's cursor; they do not implement Body admission acknowledgement.
Do not resubscribe during a guarded task to erase an unacknowledged minimum or
loss. World/session changes invalidate the task; new-world HP belongs to a new
admission. Golemkit's external integration and RTA rerun remain consumer work.

## Information ownership

1. Previously Voxrig owned receipt/native queues, Golemkit's adapter maintained
   an HP journal and completeness flag, and Body owned admission/acknowledgement.
   That flag drifted from the producer's retained evidence when an unrelated-event
   drain budget was treated as missing HP. RTA owns goals/floors, not packet history.
2. The sole authoritative history is now Voxrig's health log, written directly by
   each update_health packet reducer. Latest compatibility caches/snapshots are
   views updated after publication, never alternate history producers. Native
   all-event broadcast and common PlayerChanged notifications remain for existing
   users but do not feed this log. Golemkit derives scoped minima from its suffix.
3. Bounded eviction discards oldest records explicitly; subscriptions keep only
   a cursor and an immutable admission baseline, not a second queue. Admission
   excludes pre-cut history for that reader; accepted captures acknowledge only
   their validated cut, rejected captures retain evidence. World/session changes
   invalidate old task scope without clearing the retained audit log. Closure is
   terminal; a real gap ends the guarded task and requires fresh admission.
4. Storage is O(4096) per connection, O(1) per subscriber; append, baseline/cut,
   try_recv and minimum update are O(1). Draining k selected records costs O(k),
   independent of gameplay traffic. Missing intermediate minima cannot be derived
   from latest HP (a later 20 cannot reconstruct a lost 4), nor recovered by
   replaying unrelated gameplay. Cursor batch reads allocate O(k) for their result.

## Verification

Both adapter fixture tests run the shared contract against actual packet reducers:
30,000 non-health XP events around 20/4/20, repeated values, a 3-second unread
interval, independent readers/cuts, NaN/infinities/negative HP, real health overflow
followed by WorldChanged and disconnect. Additional tests race admission with
publication and exercise multiple async waiters, cancellation and closure. Modern
reconfiguration is covered separately.

Run the real damage/death/respawn checks against disposable loopback official
servers (the existing harness verifies Mojang SHA-1s and forwards original bytes):

```sh
cargo build --locked --example health_log_probe
python3 -B scripts/run_health_log_live.py --accept-eula --version 1.16.1 \
  --binary target/debug/examples/health_log_probe --output .local/health-log/live-1.16.1
python3 -B scripts/run_health_log_live.py --accept-eula --version 1.21.11 \
  --binary target/debug/examples/health_log_probe --output .local/health-log/live-1.21.11
```

Each LIVE report checks every original update_health payload against the public
log in order, plus actual damage, death, new-world health and terminal closure.
