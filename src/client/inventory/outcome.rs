//! Comparison-only intent and divergence evidence.
use super::{InventoryCause, InventoryDelta, InventoryLocation, InventorySnapshot};
use crate::client::{
    SlotKnowledge,
    crafting::{CraftingResultDestination, CraftingTakeRecord},
};

/// Intended effect derived only from the received pre-send basis.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryIntendedChange {
    /// Exact ordinary PICKUP/QUICK_MOVE changes. Result previews are not stock.
    Exact {
        /// Separate comparison delta; its successors are explicitly Predicted.
        delta: InventoryDelta,
    },
    /// Partial native result intention. Ingredient consumption, remainders and
    /// repeated shift craft count cannot be inferred from a displayed result.
    /// Completion conservatively yields Diverged: this is not an exact delta.
    CraftingResult {
        /// Original requested destination.
        destination: CraftingResultDestination,
        /// Displayed received output before the send.
        result: SlotKnowledge,
        /// Deterministic cursor change; material deltas are not predicted.
        cursor: InventoryDelta,
    },
}
/// A completed receive boundary requiring the caller to plan from current receipts.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InventoryDivergence {
    /// Original intent identity (may differ from the explicit resync probe's ID).
    pub send: super::InventorySendId,
    /// Immutable received values before sending, with original receipt ordinals.
    pub before: InventorySnapshot,
    /// Received values after the response/state boundary. Never a prediction.
    pub current: InventorySnapshot,
    /// Original comparison-only intention.
    pub intended: InventoryIntendedChange,
    /// Actual changes from the original received basis.
    pub actual: InventoryDelta,
    /// Received explanations; multiple hints can coexist. Empty when an
    /// ordinary send had no observed effect and no hint was received.
    pub causes: Vec<InventoryCause>,
}
/// Observed effect of a diverged send at its full received boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InventoryEffect {
    /// The server-processed boundary still equals `before` at every captured
    /// location, and every intended location was received. The send had no
    /// observed effect (for example a vanilla refusal); no change is inferred
    /// from a protocol bit such as a 1.16.1 negative comparison reply.
    NoEffect,
    /// Some received value differs from `before`, or an intended location is
    /// still unknown. Compare `actual` with `intended`.
    Different,
}
impl InventoryDivergence {
    /// Classify `actual` at the response/full-state boundary that ended the send.
    /// Derived only from received values; `causes` are not consulted.
    pub fn effect(&self) -> InventoryEffect {
        let intended = match &self.intended {
            InventoryIntendedChange::Exact { delta } => delta,
            InventoryIntendedChange::CraftingResult { cursor, .. } => cursor,
        };
        let received = intended.slots.iter().all(|slot| {
            self.current.value(slot.location).is_some_and(|v| {
                matches!(v.source, crate::client::ValueSource::Received { .. })
                    && !matches!(v.value, SlotKnowledge::Unavailable)
            })
        });
        if self.actual.slots.is_empty() && !intended.slots.is_empty() && received {
            InventoryEffect::NoEffect
        } else {
            InventoryEffect::Different
        }
    }
}
#[derive(Clone)]
pub(crate) struct Comparison {
    pub(crate) send: super::InventorySendId,
    pub(crate) before: InventorySnapshot,
    pub(crate) expected: InventorySnapshot,
    pub(crate) intended: InventoryIntendedChange,
    pub(crate) causes: Vec<InventoryCause>,
    pub(crate) interrupted: bool,
    pub(crate) crafting: bool,
    native_effects: Vec<InventoryLocation>,
}
impl Comparison {
    pub(crate) fn click(
        send: super::InventorySendId,
        record: &super::InventoryClickRecord,
        current: &crate::client::PlayerObservation,
        screen: Option<crate::client::container::ContainerScreen>,
    ) -> Self {
        let before = InventorySnapshot::capture(current, screen);
        let mut expected = before.clone();
        expected.set(
            before.screen_location(usize::from(record.source_slot)),
            record.prediction.source.value.clone(),
        );
        expected.set(
            InventoryLocation::Cursor,
            record.prediction.cursor.value.clone(),
        );
        Self::exact(send, before, expected)
    }
    pub(crate) fn transfer(
        send: super::InventorySendId,
        record: &super::InventoryTransferRecord,
        current: &crate::client::PlayerObservation,
        screen: Option<crate::client::container::ContainerScreen>,
    ) -> Self {
        let before = InventorySnapshot::capture(current, screen);
        let mut expected = before.clone();
        for slot in &record.changed_slots {
            expected.set(
                before.screen_location(usize::from(slot.slot)),
                slot.prediction.value.clone(),
            );
        }
        Self::exact(send, before, expected)
    }
    pub(crate) fn exact(
        send: super::InventorySendId,
        before: InventorySnapshot,
        expected: InventorySnapshot,
    ) -> Self {
        let intended = InventoryIntendedChange::Exact {
            delta: before.delta(&expected),
        };
        Self {
            send,
            before,
            expected,
            intended,
            causes: Vec::new(),
            interrupted: false,
            crafting: false,
            native_effects: Vec::new(),
        }
    }
    pub(crate) fn crafting(
        send: super::InventorySendId,
        record: &CraftingTakeRecord,
        current: &crate::client::PlayerObservation,
        screen: Option<crate::client::container::ContainerScreen>,
    ) -> Self {
        let before = InventorySnapshot::capture(current, screen);
        let mut expected = before.clone();
        expected.set(
            InventoryLocation::Cursor,
            record.cursor_prediction.value.clone(),
        );
        let intended = InventoryIntendedChange::CraftingResult {
            destination: record.destination,
            result: record
                .before
                .result()
                .expect("validated result")
                .value()
                .clone(),
            cursor: before.delta(&expected),
        };
        let mut native_effects = Vec::new();
        let [width, height] = record.before.dimensions();
        for y in 0..height {
            for x in 0..width {
                if let Ok((_, slot)) = record.before.input_source(x, y) {
                    native_effects.push(before.screen_location(usize::from(slot)));
                }
            }
        }
        if record.destination == CraftingResultDestination::Inventory {
            // Native repeated crafts and remainders are explicitly part of this
            // declared intention, rather than an invented exact recipe delta.
            native_effects.extend((9..=45).map(|slot| InventoryLocation::Player { slot }));
        }
        Self {
            send,
            before,
            expected,
            intended,
            causes: Vec::new(),
            interrupted: false,
            crafting: true,
            native_effects,
        }
    }
    pub(crate) fn hint(&mut self, cause: InventoryCause) {
        if !self.causes.contains(&cause) {
            self.causes.push(cause);
        }
    }
    pub(crate) fn inspect(
        &mut self,
        current: &InventorySnapshot,
        crafting_complete: bool,
        ready: bool,
    ) -> bool {
        if self.before.session != current.session || self.before.source != current.source {
            return false;
        }
        let before = self.before.locations();
        // expected is never used as received evidence; access its predicted fields directly.
        let mut equal = true;
        for (location, predecessor) in before {
            // A result preview is not an ordinary click's physical effect.
            if self.before.preview(location) {
                continue;
            }
            let predicted = self.expected.value(location);
            let now = current
                .value(location)
                .filter(|v| matches!(v.source, crate::client::ValueSource::Received { .. }));
            if self.native_effects.contains(&location) {
                // Native grid/result take semantics are checked by the original
                // strict crafting record, rather than a fabricated recipe model.
                continue;
            }
            let value = now.map(|v| &v.value);
            let expected = predicted.map(|v| &v.value);
            let previous = predecessor.as_ref().map(|v| &v.value);
            if previous.is_none() {
                let delta = match &self.intended {
                    InventoryIntendedChange::Exact { delta } => delta,
                    InventoryIntendedChange::CraftingResult { cursor, .. } => cursor,
                };
                if !delta.slots.iter().any(|slot| slot.location == location) {
                    // An unrelated first receipt hydrates knowledge, not stock.
                    // Keep the actual receipt/delta intact; only comparison is exempt.
                    continue;
                }
                // An intended location with no received predecessor has no
                // prediction basis. Keep it conservative even if the first
                // receipt happens to match a guessed successor; hydration
                // cannot establish that this send caused the intended change.
                equal = false;
            }
            if value != expected {
                equal = false;
            }
            if previous.is_some() && value != previous && value != expected {
                self.interrupted = true;
                self.hint(InventoryCause::External);
            }
        }
        if ready && self.crafting && !crafting_complete {
            equal = false;
        }
        // Fresh native grid/result receipts establish delivery, not agreement
        // with an exact pre-send material/slot delta. In particular, a /give in
        // the native shift destination range must not be certified as Received.
        // Until that full intention is known, even an otherwise ordinary craft
        // requires the planner to use the newly received inventory.
        equal && !self.interrupted && !self.crafting
    }
    pub(crate) fn divergence(&self, current: InventorySnapshot) -> InventoryDivergence {
        let mut divergence = InventoryDivergence {
            send: self.send,
            before: self.before.clone(),
            actual: self.before.delta(&current),
            current,
            intended: self.intended.clone(),
            causes: self.causes.clone(),
        };
        // An unchanged full boundary is the caller's own no-effect send, not an
        // external change. Crafting keeps its fallback (strict take record owns it).
        let unchanged = !self.crafting && divergence.effect() == InventoryEffect::NoEffect;
        if divergence.causes.is_empty() && !unchanged {
            divergence.causes.push(InventoryCause::External);
        }
        divergence
    }
}

impl crate::Client {
    /// Comparison evidence for the retained send (or its timed-out original after resync).
    /// Returns None until Diverged. Retain the value before sending again; a
    /// superseded send or retired world/opening returns State, never another send's data.
    pub async fn inventory_divergence(
        &self,
        send: super::InventorySendId,
    ) -> crate::Result<Option<InventoryDivergence>> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::EventOps::inventory_divergence(a, send).await)
    }
}
