//! Own non-executor actions observed at the socket writer (I9).
//!
//! Item use, placement, digging, selected drop and hand swap are not inventory
//! executor sends, yet the server answers them with ordinary slot packets. The
//! writer records each such frame before it is written; the receive path then
//! opens a bounded window scoped to the hand slots that action can change. A
//! delta inside that window and scope is tagged `OwnAction`, which, like
//! `Send`, is a correlation hint and never proof of causal ownership.
use super::InventoryLocation;
use crate::MinecraftVersion;
use crate::client::SessionStamp;
use std::collections::VecDeque;
use tokio::time::{Duration, Instant};

/// How long after its write an own action can explain a hand-slot delta.
///
/// Consumption at the end of eating or drinking is server-tick driven and can
/// arrive after the 1.21.11 interaction ACK, so the window is a fixed deadline
/// covering a full 32-tick use plus latency rather than closing at the ACK.
pub const INVENTORY_OWN_ACTION_WINDOW: Duration = Duration::from_secs(3);
// Bounded retention independent of the deadline (one tick of rapid writes).
const MAX_OWN_ACTIONS: usize = 64;

/// Kind of own action whose write opened the window.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryOwnActionKind {
    /// Use item (eat, drink, throw, bucket use in air).
    UseItem,
    /// Use item on a block, including placement and bucket fill.
    UseOnBlock,
    /// Start or finish of a block break (tool durability).
    Dig,
    /// Release of a used item (bow, crossbow, trident).
    ReleaseUseItem,
    /// Drop one item or the whole selected stack.
    DropSelected,
    /// Swap main and off hand.
    SwapHands,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    Main,
    Off,
    Both,
}
#[derive(Clone, Debug)]
struct Pending {
    kind: InventoryOwnActionKind,
    scope: Scope,
    interaction_sequence: Option<i32>,
    written: Instant,
    held_writes: u64,
    last_held: Option<u8>,
}
#[derive(Default)]
struct Queue {
    version: Option<MinecraftVersion>,
    held_writes: u64,
    last_held: Option<u8>,
    pending: VecDeque<Pending>,
}
/// Writer-to-receiver handoff. A plain mutex never held across an await, so
/// the writer cannot wait on a receive path holding inventory/state locks.
#[derive(Default)]
pub(crate) struct OwnActionSink(std::sync::Mutex<Queue>);
impl OwnActionSink {
    pub(crate) fn attach(&self, version: MinecraftVersion) {
        let mut queue = self.lock();
        *queue = Queue {
            version: Some(version),
            ..Default::default()
        };
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Classify one serverbound play frame before it is written.
    pub(crate) fn observe(&self, id: i32, payload: &[u8]) {
        let mut queue = self.lock();
        let Some(version) = queue.version else {
            return;
        };
        let Some(frame) = classify(version, id, payload) else {
            return;
        };
        match frame {
            Frame::Held(slot) => {
                queue.held_writes += 1;
                queue.last_held = Some(slot);
            }
            Frame::Action {
                kind,
                scope,
                interaction_sequence,
            } => {
                let pending = Pending {
                    kind,
                    scope,
                    interaction_sequence,
                    written: Instant::now(),
                    held_writes: queue.held_writes,
                    last_held: queue.last_held,
                };
                queue.pending.push_back(pending);
                if queue.pending.len() > MAX_OWN_ACTIONS {
                    queue.pending.pop_front();
                }
            }
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.lock().pending.is_empty()
    }
    /// Resolve pending writes against the current received/submitted selection.
    /// A selection written after the action makes `selected` ambiguous for it;
    /// then the selection known at write time is used, or the main hand is
    /// left out of scope rather than guessed.
    pub(crate) fn drain(
        &self,
        session: SessionStamp,
        selected: Option<u8>,
        next_ordinal: &mut u64,
    ) -> Vec<OwnActionWindow> {
        let mut queue = self.lock();
        let held_writes = queue.held_writes;
        queue
            .pending
            .drain(..)
            .map(|p| {
                let selected = if p.held_writes == held_writes {
                    selected
                } else {
                    p.last_held
                };
                let main = selected
                    .filter(|s| *s < 9)
                    .map(|s| InventoryLocation::Player {
                        slot: 36 + usize::from(s),
                    });
                let off = InventoryLocation::Player { slot: 45 };
                let scope = match p.scope {
                    Scope::Main => main.into_iter().collect(),
                    Scope::Off => vec![off],
                    Scope::Both => main.into_iter().chain([off]).collect(),
                };
                *next_ordinal += 1;
                OwnActionWindow {
                    session,
                    kind: p.kind,
                    ordinal: *next_ordinal,
                    interaction_sequence: p.interaction_sequence,
                    scope,
                    deadline: p.written + INVENTORY_OWN_ACTION_WINDOW,
                }
            })
            .collect()
    }
}

/// One bounded own-action window, resolved to physical player slots.
#[derive(Clone, Debug)]
pub(crate) struct OwnActionWindow {
    pub(crate) session: SessionStamp,
    pub(crate) kind: InventoryOwnActionKind,
    pub(crate) ordinal: u64,
    pub(crate) interaction_sequence: Option<i32>,
    pub(crate) scope: Vec<InventoryLocation>,
    pub(crate) deadline: Instant,
}

enum Frame {
    Held(u8),
    Action {
        kind: InventoryOwnActionKind,
        scope: Scope,
        interaction_sequence: Option<i32>,
    },
}
fn varint(bytes: &mut &[u8]) -> Option<i32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value as i32);
        }
    }
    None
}
fn skip(bytes: &mut &[u8], n: usize) -> Option<()> {
    *bytes = bytes.get(n..)?;
    Some(())
}
fn hand(value: i32) -> Option<Scope> {
    match value {
        0 => Some(Scope::Main),
        1 => Some(Scope::Off),
        _ => None,
    }
}
// Native serverbound layouts: 1.16.1 (736) has no interaction sequence;
// 1.21.11 (774) carries one on use, use-on-block and player action.
fn classify(version: MinecraftVersion, id: i32, payload: &[u8]) -> Option<Frame> {
    use InventoryOwnActionKind as K;
    let modern = version == MinecraftVersion::Java1_21_11;
    let (held, dig, place, use_item) = if modern {
        (0x34, 0x28, 0x3f, 0x40)
    } else {
        (0x24, 0x1b, 0x2d, 0x2e)
    };
    let mut bytes = payload;
    let sequence = |bytes: &mut &[u8]| if modern { varint(bytes) } else { None };
    if id == held {
        let slot = i16::from_be_bytes(payload.get(..2)?.try_into().ok()?);
        return u8::try_from(slot).ok().map(Frame::Held);
    }
    let (kind, scope, interaction_sequence) = if id == dig {
        let status = varint(&mut bytes)?;
        let (kind, scope) = match status {
            0 | 2 => (K::Dig, Scope::Main),
            3 | 4 => (K::DropSelected, Scope::Main),
            5 => (K::ReleaseUseItem, Scope::Both),
            6 => (K::SwapHands, Scope::Both),
            _ => return None,
        };
        skip(&mut bytes, 9)?;
        (kind, scope, sequence(&mut bytes))
    } else if id == place {
        let scope = hand(varint(&mut bytes)?)?;
        skip(&mut bytes, 8)?;
        varint(&mut bytes)?;
        skip(&mut bytes, 12 + if modern { 2 } else { 1 })?;
        (K::UseOnBlock, scope, sequence(&mut bytes))
    } else if id == use_item {
        let scope = hand(varint(&mut bytes)?)?;
        (K::UseItem, scope, sequence(&mut bytes))
    } else {
        return None;
    };
    Some(Frame::Action {
        kind,
        scope,
        interaction_sequence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_varint(out: &mut Vec<u8>, mut value: i32) {
        loop {
            let byte = (value & 0x7f) as u8;
            value = ((value as u32) >> 7) as i32;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }
    fn session(version: MinecraftVersion) -> SessionStamp {
        SessionStamp {
            version,
            connection_id: 3,
            world_generation: 1,
        }
    }

    #[test]
    fn classifies_native_frames_with_scope_and_sequence() {
        let modern = MinecraftVersion::Java1_21_11;
        let sink = OwnActionSink::default();
        sink.observe(0x40, &[0, 7]);
        assert!(sink.is_empty(), "unattached writer records nothing");
        sink.attach(modern);
        // use item, off hand, sequence 300
        let mut use_item = vec![1];
        put_varint(&mut use_item, 300);
        use_item.extend([0; 8]);
        sink.observe(0x40, &use_item);
        // use on block, main hand, sequence 9
        let mut place = vec![0];
        place.extend([0; 8]);
        put_varint(&mut place, 1);
        place.extend([0; 14]);
        put_varint(&mut place, 9);
        sink.observe(0x3f, &place);
        // abort dig is ignored; drop and swap are recorded
        for status in [1, 4, 6] {
            let mut dig = vec![status];
            dig.extend([0; 9]);
            put_varint(&mut dig, 10 + i32::from(status));
            sink.observe(0x28, &dig);
        }
        sink.observe(0x09, &[0; 4]);
        let mut ordinal = 0;
        let windows = sink.drain(session(modern), Some(2), &mut ordinal);
        let got: Vec<_> = windows
            .iter()
            .map(|w| (w.kind, w.interaction_sequence, w.scope.clone(), w.ordinal))
            .collect();
        let main = InventoryLocation::Player { slot: 38 };
        let off = InventoryLocation::Player { slot: 45 };
        assert_eq!(
            got,
            [
                (InventoryOwnActionKind::UseItem, Some(300), vec![off], 1),
                (InventoryOwnActionKind::UseOnBlock, Some(9), vec![main], 2),
                (
                    InventoryOwnActionKind::DropSelected,
                    Some(14),
                    vec![main],
                    3
                ),
                (
                    InventoryOwnActionKind::SwapHands,
                    Some(16),
                    vec![main, off],
                    4
                ),
            ]
        );
        assert!(sink.is_empty());
    }

    #[test]
    fn legacy_frames_have_no_sequence_and_later_selection_is_not_guessed() {
        let legacy = MinecraftVersion::Java1_16_1;
        let sink = OwnActionSink::default();
        sink.attach(legacy);
        sink.observe(0x2e, &[0]);
        sink.observe(0x24, &4i16.to_be_bytes());
        let mut place = vec![0];
        place.extend([0; 8]);
        place.push(1);
        place.extend([0; 13]);
        sink.observe(0x2d, &place);
        let mut ordinal = 5;
        let windows = sink.drain(session(legacy), Some(4), &mut ordinal);
        // The use was written before selection 4 with no earlier own selection:
        // its main hand is unknown, never assumed to be the current slot.
        assert_eq!(windows[0].kind, InventoryOwnActionKind::UseItem);
        assert!(windows[0].scope.is_empty());
        assert_eq!(windows[0].interaction_sequence, None);
        assert_eq!(windows[1].kind, InventoryOwnActionKind::UseOnBlock);
        assert_eq!(windows[1].scope, [InventoryLocation::Player { slot: 40 }]);
        assert_eq!((windows[0].ordinal, windows[1].ordinal), (6, 7));
    }

    fn snapshot(
        session: SessionStamp,
        sequence: u64,
        slot45: u32,
    ) -> super::super::InventorySnapshot {
        let mut slots = vec![
            Some(crate::client::received(
                crate::client::SlotKnowledge::Empty,
                1
            ));
            46
        ];
        let item = crate::client::registry::Registry::for_version(session.version)
            .item("minecraft:snowball")
            .unwrap();
        slots[45] = Some(crate::client::received(
            crate::client::SlotKnowledge::Item {
                item: crate::client::ItemStack {
                    id: item.id,
                    name: item.name,
                    count: slot45,
                    data: crate::client::ItemData::Default,
                },
            },
            sequence,
        ));
        super::super::InventorySnapshot::from_inventory(
            session,
            sequence,
            &crate::client::InventoryObservation {
                slots,
                cursor: Some(crate::client::received(
                    crate::client::SlotKnowledge::Empty,
                    1,
                )),
                window_id: Some(0),
                ..Default::default()
            },
            None,
        )
    }

    #[test]
    fn expired_or_other_world_windows_leave_external() {
        use super::super::{InventoryCause, changes::InventoryChangeLedger};
        let stamp = session(MinecraftVersion::Java1_16_1);
        let mut next_world = stamp;
        next_world.world_generation += 1;
        let window = |session, deadline| OwnActionWindow {
            session,
            kind: InventoryOwnActionKind::UseItem,
            ordinal: 1,
            interaction_sequence: None,
            scope: vec![InventoryLocation::Player { slot: 45 }],
            deadline,
        };
        let later = Instant::now() + Duration::from_secs(60);
        for (open, expected_own) in [
            (window(stamp, later), true),
            (window(stamp, Instant::now()), false),
            (window(next_world, later), false),
        ] {
            let mut ledger = InventoryChangeLedger::default();
            ledger.receive(snapshot(stamp, 1, 4), None, &[]);
            ledger.own_actions([open]);
            ledger.receive(snapshot(stamp, 2, 3), None, &[]);
            let causes = &ledger.after(0, 2).unwrap().changes[0].causes;
            assert_eq!(
                causes.contains(&InventoryCause::External),
                !expected_own,
                "{causes:?}"
            );
            assert_eq!(
                causes
                    .iter()
                    .any(|c| matches!(c, InventoryCause::OwnAction { .. })),
                expected_own
            );
        }
        // A world change discards windows even when the deadline is far away.
        let mut ledger = InventoryChangeLedger::default();
        ledger.receive(snapshot(stamp, 1, 4), None, &[]);
        ledger.own_actions([window(stamp, later)]);
        ledger.receive(snapshot(next_world, 2, 4), None, &[]);
        ledger.receive(snapshot(next_world, 3, 3), None, &[]);
        assert_eq!(
            ledger.after(2, 3).unwrap().changes[0].causes,
            [InventoryCause::External]
        );
    }
}
