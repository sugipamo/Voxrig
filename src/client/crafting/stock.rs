//! Original recipe-book inventory accounting, distinct from manual crafting.
use crate::client::{
    ReceivedItem, SessionStamp,
    registry::{RegistryId, ServerRegistryStamp},
};
use crate::{MinecraftVersion, Result};

/// Native recipe-book facts for one immutable received inventory stack.
/// This is neither ingredient membership nor permission to place/craft a recipe.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipeBookStock {
    session: SessionStamp,
    registry_owner: ServerRegistryStamp,
    receive_sequence: u64,
    item: RegistryId,
    damaged: bool,
    enchanted: bool,
    custom_named: bool,
    count: i32,
}
impl RecipeBookStock {
    /// Transport/world at the original inventory capture.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Registry configuration owning the received stack.
    pub fn registry_owner(&self) -> ServerRegistryStamp {
        self.registry_owner
    }
    /// Actual packet supplying the stack, not a later freshness guarantee.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Item type in the selected version's registry.
    pub fn item(&self) -> RegistryId {
        self.item
    }
    /// Native damaged predicate, including damageability and unbreakable rules.
    pub fn damaged(&self) -> bool {
        self.damaged
    }
    /// Native enchantments predicate; stored enchantments/glint alone do not imply it.
    pub fn enchanted(&self) -> bool {
        self.enchanted
    }
    /// Native custom-name presence/type predicate; not rendered text or item-name.
    pub fn custom_named(&self) -> bool {
        self.custom_named
    }
    /// Native simple-stock filter. This does not prove a stack matches an ingredient.
    pub fn eligible(&self) -> bool {
        !self.damaged && !self.enchanted && !self.custom_named
    }
    /// Signed native contribution to inventory recipe-book stock accounting.
    /// Legacy caps each admitted stack at 64; modern uses effective stack capacity.
    /// Untrusted modern capacity can produce zero/negative contribution even when
    /// eligible. It is preserved here, not turned into available crafting material.
    pub fn count(&self) -> i32 {
        self.count
    }
    pub(crate) fn capture(received: &ReceivedItem<'_>) -> Result<Self> {
        let item = received.stack();
        let properties = item.properties()?;
        let (enchanted, custom_named) = match item.id.version() {
            MinecraftVersion::Java1_16_1 => {
                let data = item.custom_data()?;
                let root = data.as_ref().map(|v| v.root());
                let enchanted = root
                    .and_then(|r| r.get("Enchantments"))
                    .and_then(|v| v.as_list())
                    .is_some_and(|values| {
                        values.first().is_some_and(|v| v.as_compound().is_some())
                    });
                let custom_named = root
                    .and_then(|r| r.get("display"))
                    .and_then(|v| v.as_compound())
                    .and_then(|r| r.get("Name"))
                    .is_some_and(|v| v.as_string().is_some());
                (enchanted, custom_named)
            }
            MinecraftVersion::Java1_21_11 => {
                crate::versions::java_1_21_11::item_components::recipe_book_flags(item)
                    .map_err(|e| crate::Error::new(crate::ErrorKind::InvalidInput, e))?
            }
        };
        let eligible = !properties.damaged && !enchanted && !custom_named;
        let cap = match item.id.version() {
            MinecraftVersion::Java1_16_1 => 64,
            MinecraftVersion::Java1_21_11 => properties.max_stack_size,
        };
        let count = if eligible {
            i32::try_from(item.count)
                .map_err(|e| crate::Error::new(crate::ErrorKind::InvalidInput, e))?
                .min(cap)
        } else {
            0
        };
        Ok(Self {
            session: received.registry_state().session(),
            registry_owner: received.registry_state().stamp(),
            receive_sequence: received.receive_sequence(),
            item: item.id,
            damaged: properties.damaged,
            enchanted,
            custom_named,
            count,
        })
    }
}
#[cfg(test)]
mod tests;
