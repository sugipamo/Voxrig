//! Received native container data, separate from slot contents and click authority.
use super::{ObservedValue, ScreenId, SessionStamp, received};
use std::collections::BTreeMap;

/// Maximum distinct native properties retained per opening; oldest receipts are evicted.
pub const MAX_SCREEN_PROPERTIES: usize = 256;

/// Native data for one received opening, without local time extrapolation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScreenProperties {
    /// Connection/world/opening identity shared with `ContainerScreen`.
    pub id: ScreenId,
    /// Native signed short property IDs and values, each with its own receive ordinal.
    /// Missing keys mean no retained receipt; a received zero remains an explicit value.
    pub values: BTreeMap<i16, ObservedValue<i16>>,
    /// At least one distinct property was evicted during this opening.
    /// When true, a missing key may have been received before eviction.
    pub evicted: bool,
}
/// Coherent read-only capture of current native container properties.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScreenPropertyObservation {
    /// Connection/world at capture.
    pub session: SessionStamp,
    /// Current applied packet ordinal, independent of individual property freshness.
    pub receive_sequence: u64,
    /// None until an OPEN is received, and after received close or world reset.
    pub screen: Option<ScreenProperties>,
}
impl crate::Client {
    /// Read Java 1.21.11 container data for the current received opening.
    /// Sends no packets. Java 1.16.1 returns `Unsupported`; closed connections error.
    /// Same-value receipts advance only that property's ordinal. No progress is
    /// generated from local time, and notifications alone do not contain these values.
    pub async fn screen_properties(&self) -> crate::Result<ScreenPropertyObservation> {
        crate::client::dispatch!(&self.adapter, a => crate::client::adapter::CoreOps::screen_properties(a).await)
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct PropertyReceipts {
    pub values: BTreeMap<i16, ObservedValue<i16>>,
    pub evicted: bool,
}
impl PropertyReceipts {
    pub fn receive(&mut self, property: i16, value: i16, sequence: u64) {
        if !self.values.contains_key(&property) && self.values.len() == MAX_SCREEN_PROPERTIES {
            let oldest = *self
                .values
                .iter()
                .min_by_key(|(_, v)| match v.source {
                    crate::client::ValueSource::Received { sequence } => sequence,
                    _ => unreachable!("properties only retain actual receipts"),
                })
                .expect("nonempty bounded properties")
                .0;
            self.values.remove(&oldest);
            self.evicted = true;
        }
        self.values.insert(property, received(value, sequence));
    }
}
