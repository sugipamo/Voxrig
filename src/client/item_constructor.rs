//! Common internal native item constructor fields. Registry references inside
//! an adapter's patch still require their owning live context before comparison.
use super::{fraction::Fraction, registry::RegistryId};
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct Item<P> {
    pub count: i32,
    pub native_id: Option<RegistryId>,
    pub patch: Option<P>,
}
impl<P> Item<P> {
    pub(crate) fn empty() -> Self {
        Self {
            count: 0,
            native_id: None,
            patch: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub(crate) struct Bundle<P> {
    pub items: Vec<Item<P>>,
    pub weight: Fraction,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WeightFields {
    pub max_stack_size: i32,
    pub bundle: Option<Fraction>,
    pub has_bees: bool,
}
impl WeightFields {
    pub(crate) fn weight(self, count: i32) -> Result<Fraction> {
        let unit = if let Some(contents) = self.bundle {
            Fraction::new(1, 16)?.add(contents)?
        } else if self.has_bees {
            Fraction::new(1, 1)?
        } else {
            Fraction::new(1, self.max_stack_size)?
        };
        unit.multiply(Fraction::new(count, 1)?)
    }
}
