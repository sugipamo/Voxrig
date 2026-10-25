//! Private version-neutral evidence; never synthesized from cache equality.
use super::{self as body, *};
use std::collections::VecDeque;
#[derive(Clone)]
pub(crate) enum Update {
    Block([i32; 3], i32),
    DigAck {
        pos: [i32; 3],
        state: i32,
        status: i32,
        successful: bool,
    },
    Pose {
        position: [f64; 3],
        rotation: [f32; 2],
    },
    Selection(u8),
    Resync,
    World,
    Chunk([i32; 2]),
}
#[derive(Clone)]
pub(crate) struct Evidence {
    pub ordinal: u64,
    pub sequence: u64,
    pub update: Update,
}
#[derive(Default)]
pub(crate) struct EvidenceLog {
    pub(crate) events: VecDeque<Evidence>,
    pub(crate) next: u64,
    pub(crate) dropped: u64,
}
impl EvidenceLog {
    pub(crate) fn push(&mut self, sequence: u64, update: Update) {
        self.next += 1;
        self.events.push_back(Evidence {
            ordinal: self.next,
            sequence,
            update,
        });
        if self.events.len() > 4096 {
            self.dropped = self.events.pop_front().unwrap().ordinal;
        }
    }
}
pub(crate) fn source(source: crate::client::ValueSource) -> body::ValueSource {
    use crate::client::ValueSource as S;
    match source {
        S::Received { sequence } => body::ValueSource::Received { sequence },
        S::Submitted => body::ValueSource::Submitted,
        S::Predicted => body::ValueSource::Predicted,
        S::LocalCache => body::ValueSource::LocalCache,
    }
}
pub(crate) fn value<T>(v: crate::client::ObservedValue<T>) -> Value<T> {
    Value {
        value: v.value,
        source: source(v.source),
    }
}
pub(crate) fn slot(
    v: crate::client::ObservedValue<crate::client::SlotKnowledge>,
) -> Value<BodySlot> {
    Value {
        source: source(v.source),
        value: match v.value {
            crate::client::SlotKnowledge::Unavailable => BodySlot::Unavailable,
            crate::client::SlotKnowledge::Empty => BodySlot::Empty,
            crate::client::SlotKnowledge::Item { item } => BodySlot::Item(body::BodyItem {
                name: item.name,
                count: item.count,
                data: Some(item.data),
            }),
        },
    }
}
pub(crate) struct EvidenceBatch {
    pub cursor: u64,
    pub gap: bool,
    pub events: Vec<Evidence>,
    pub ack: Option<i32>,
    pub snapshot: BodySnapshot,
}
