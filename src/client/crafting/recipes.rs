//! Immutable server recipe/display catalogue; display is not inventory authority.
use crate::Result;
use crate::client::{
    ItemStack, SessionStamp, SlotKnowledge,
    nbt::NbtValue,
    registry::{
        ItemDefinition, Registry, RegistryId, ServerRegistryId, ServerRegistryObservation,
        ServerRegistryStamp,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn invalid(message: impl std::fmt::Display) -> crate::Error {
    super::super::inventory::unavailable(message)
}

/// Display-only inline trim pattern. It does not describe a received armor item.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipeTrimDefinition {
    /// Native asset identifier.
    pub asset: String,
    /// Actual component stream's decoded unnamed NBT; not an inventory receipt.
    pub description: Arc<NbtValue>,
    /// Native decal flag.
    pub decal: bool,
}
/// A trim preview pattern with its actual received registry binding.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RecipeTrimPattern {
    #[serde(skip)]
    pub(crate) raw_reference: Option<i32>,
    reference: Option<ServerRegistryId>,
    inline: Option<RecipeTrimDefinition>,
}
impl RecipeTrimPattern {
    /// Bound server registry reference, absent for an inline pattern.
    pub fn reference(&self) -> Option<&ServerRegistryId> {
        self.reference.as_ref()
    }
    /// Native inline display definition, absent for a registry reference.
    pub fn inline_definition(&self) -> Option<&RecipeTrimDefinition> {
        self.inline.as_ref()
    }
    pub(crate) fn raw_reference(value: i32) -> Self {
        Self {
            raw_reference: Some(value),
            reference: None,
            inline: None,
        }
    }
    pub(crate) fn inline(value: RecipeTrimDefinition) -> Self {
        Self {
            raw_reference: None,
            reference: None,
            inline: Some(value),
        }
    }
}
/// Server-supplied visual ingredient/result choices. These are not slot receipts,
/// crafting permission, predicted remainders or an exact crafted output.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecipeSlotDisplay {
    /// Native empty display.
    Empty,
    /// Fuel choices need the owning world's fuel context.
    AnyFuel,
    /// One item type with its selected-version ID.
    Item {
        /// Selected-version item definition.
        item: ItemDefinition,
    },
    /// One display stack, including native data; not received inventory.
    Stack {
        /// Native display stack.
        item: ItemStack,
    },
    /// Resolve through this catalogue's actual item tags.
    Tag {
        /// Namespaced item tag.
        name: String,
    },
    /// Original native display alternatives, in supplied order.
    Composite {
        /// Native alternatives, which may be empty.
        contents: Vec<RecipeSlotDisplay>,
    },
    /// Displayed input/remainder association, not observed consumption.
    WithRemainder {
        /// Displayed input.
        input: Box<RecipeSlotDisplay>,
        /// Displayed remainder.
        remainder: Box<RecipeSlotDisplay>,
    },
    /// Demonstration of smithing trim; exact result still depends on the base item.
    TrimPreview {
        /// Displayed base choices.
        base: Box<RecipeSlotDisplay>,
        /// Displayed material choices.
        material: Box<RecipeSlotDisplay>,
        /// Bound or inline trim pattern.
        pattern: RecipeTrimPattern,
    },
}
impl RecipeSlotDisplay {
    fn bind(&mut self, owner: &ServerRegistryObservation) -> Result<()> {
        match self {
            Self::Item { item } => {
                let actual = Registry::for_version(owner.session().version)
                    .item_by_native_id(item.id.value())?;
                if actual != *item {
                    return Err(invalid("display item belongs to another registry"));
                }
            }
            Self::Stack { item } => {
                let actual = Registry::for_version(owner.session().version)
                    .item_by_native_id(item.id.value())?;
                if actual.id != item.id || actual.name != item.name {
                    return Err(invalid("display stack belongs to another registry"));
                }
            }
            Self::Composite { contents } => {
                for v in contents {
                    v.bind(owner)?;
                }
            }
            Self::WithRemainder { input, remainder } => {
                input.bind(owner)?;
                remainder.bind(owner)?;
            }
            Self::TrimPreview {
                base,
                material,
                pattern,
            } => {
                base.bind(owner)?;
                material.bind(owner)?;
                if let Some(id) = pattern.raw_reference {
                    pattern.reference = Some(owner.bind("minecraft:trim_pattern", id)?);
                }
            }
            Self::Empty | Self::AnyFuel | Self::Tag { .. } => {}
        }
        Ok(())
    }
    pub(super) fn item_ids(&self, owner: &ServerRegistryObservation) -> Result<Vec<RegistryId>> {
        Ok(match self {
            Self::Empty => vec![],
            Self::Item { item } => vec![item.id],
            Self::Stack { item } => vec![item.id],
            Self::Tag { name } => tag_items(owner, name)?,
            Self::Composite { contents } => {
                let mut ids = Vec::new();
                for v in contents {
                    ids.extend(v.item_ids(owner)?);
                }
                ids
            }
            Self::WithRemainder { input, .. } => input.item_ids(owner)?,
            Self::TrimPreview { base, .. } => base.item_ids(owner)?,
            Self::AnyFuel => {
                return Err(invalid(
                    "native fuel context is not yet captured by recipe catalogue",
                ));
            }
        })
    }
}
fn tag_items(owner: &ServerRegistryObservation, name: &str) -> Result<Vec<RegistryId>> {
    let tags = owner
        .tags()
        .ok_or_else(|| invalid("recipe item tags have not been received"))?;
    tags.value
        .get("minecraft:item")
        .and_then(|tags| tags.get(name))
        .into_iter()
        .flatten()
        .map(|&id| {
            Registry::for_version(owner.session().version)
                .item_by_native_id(id)
                .map(|item| item.id)
        })
        .collect()
}
/// Item membership supplied by the recipe, separate from display stacks/counts.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecipeIngredient {
    /// Native item alternatives. Each ingredient still requires a separate unit.
    Items {
        /// IDs belong to the selected item registry.
        items: Vec<RegistryId>,
    },
    /// Native named item set, resolved against actual captured tags.
    Tag {
        /// Namespaced tag.
        name: String,
    },
}
impl RecipeIngredient {
    /// Resolve item membership using this immutable received catalogue.
    /// Missing tag receipts are an error; a received absent tag yields no members.
    pub fn items(&self, catalogue: &ReceivedRecipes) -> Result<Vec<RegistryId>> {
        match self {
            Self::Items { items } => {
                let registry = Registry::for_version(catalogue.session.version);
                items
                    .iter()
                    .map(|id| registry.item_definition(*id).map(|v| v.id))
                    .collect()
            }
            Self::Tag { name } => tag_items(&catalogue.registries, name),
        }
    }
}
/// Received recipe display/topology. Legacy lacks a crafting-station display.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecipeDisplay {
    /// Shaped arrangement, preserving original integer dimensions.
    Shaped {
        /// Native width, not yet a validated execution grid size.
        width: i32,
        /// Native height.
        height: i32,
        /// Row-major native displays, including empty cells.
        ingredients: Vec<RecipeSlotDisplay>,
        /// Displayed result.
        result: RecipeSlotDisplay,
        /// Actual station display if the version transmits it.
        crafting_station: Option<RecipeSlotDisplay>,
    },
    /// Ingredients have no recipe-defined coordinates.
    Shapeless {
        /// Native ingredient displays.
        ingredients: Vec<RecipeSlotDisplay>,
        /// Displayed result.
        result: RecipeSlotDisplay,
        /// Actual station display when transmitted.
        crafting_station: Option<RecipeSlotDisplay>,
    },
    /// Cooking display. Legacy transmits its exact cooking serializer name.
    Furnace {
        /// Legacy serializer name, or modern generic furnace display type.
        recipe_kind: String,
        /// Input display.
        ingredient: RecipeSlotDisplay,
        /// Actual fuel display, absent on legacy.
        fuel: Option<RecipeSlotDisplay>,
        /// Result display.
        result: RecipeSlotDisplay,
        /// Actual station display when transmitted.
        crafting_station: Option<RecipeSlotDisplay>,
        /// Native duration.
        duration: i32,
        /// IEEE-754 experience bits, preserving wire values.
        experience_bits: u32,
    },
    /// Stonecutting display.
    Stonecutter {
        /// Input display.
        input: RecipeSlotDisplay,
        /// Result display.
        result: RecipeSlotDisplay,
        /// Actual station display when transmitted.
        crafting_station: Option<RecipeSlotDisplay>,
    },
    /// Legacy smithing has no template; modern supplies one.
    Smithing {
        /// Actual template display when transmitted.
        template: Option<RecipeSlotDisplay>,
        /// Base display.
        base: RecipeSlotDisplay,
        /// Addition display.
        addition: RecipeSlotDisplay,
        /// Result display.
        result: RecipeSlotDisplay,
        /// Actual station display when transmitted.
        crafting_station: Option<RecipeSlotDisplay>,
    },
    /// A declaration whose special algorithm/result is not transmitted.
    Special {
        /// Exact received serializer name.
        recipe_kind: String,
    },
}
impl RecipeDisplay {
    /// Displayed result, absent when this version does not transmit one.
    pub fn result(&self) -> Option<&RecipeSlotDisplay> {
        match self {
            Self::Shaped { result, .. }
            | Self::Shapeless { result, .. }
            | Self::Furnace { result, .. }
            | Self::Stonecutter { result, .. }
            | Self::Smithing { result, .. } => Some(result),
            Self::Special { .. } => None,
        }
    }
    pub(crate) fn bind(&mut self, owner: &ServerRegistryObservation) -> Result<()> {
        match self {
            Self::Shaped {
                ingredients,
                result,
                crafting_station,
                ..
            }
            | Self::Shapeless {
                ingredients,
                result,
                crafting_station,
            } => {
                for v in ingredients {
                    v.bind(owner)?;
                }
                result.bind(owner)?;
                if let Some(v) = crafting_station {
                    v.bind(owner)?;
                }
            }
            Self::Furnace {
                ingredient,
                fuel,
                result,
                crafting_station,
                ..
            } => {
                ingredient.bind(owner)?;
                if let Some(v) = fuel {
                    v.bind(owner)?;
                }
                result.bind(owner)?;
                if let Some(v) = crafting_station {
                    v.bind(owner)?;
                }
            }
            Self::Stonecutter {
                input,
                result,
                crafting_station,
            } => {
                input.bind(owner)?;
                result.bind(owner)?;
                if let Some(v) = crafting_station {
                    v.bind(owner)?;
                }
            }
            Self::Smithing {
                template,
                base,
                addition,
                result,
                crafting_station,
            } => {
                if let Some(v) = template {
                    v.bind(owner)?;
                }
                base.bind(owner)?;
                addition.bind(owner)?;
                result.bind(owner)?;
                if let Some(v) = crafting_station {
                    v.bind(owner)?;
                }
            }
            Self::Special { .. } => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum NativeRecipeId {
    Legacy(String),
    Modern(i32),
}
/// Opaque server recipe identity, valid only for this entry/connection/configuration.
/// Serializing it is diagnostic; no deserialization or constructor grants authority.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RecipeId {
    owner: ServerRegistryStamp,
    entry_sequence: u64,
    native: NativeRecipeId,
}
impl RecipeId {
    pub(crate) fn declared_ghost(name: String, ordinal: u64, owner: ServerRegistryStamp) -> Self {
        Self {
            owner,
            entry_sequence: ordinal,
            native: NativeRecipeId::Legacy(name),
        }
    }
    pub(crate) fn native(&self) -> &NativeRecipeId {
        &self.native
    }
    /// Original registry configuration/transport.
    pub fn owner(&self) -> ServerRegistryStamp {
        self.owner
    }
    /// Actual declaration/add ordinal supplying this entry.
    pub fn receive_sequence(&self) -> u64 {
        self.entry_sequence
    }
}
/// Immutable recipe entry with received declaration/book provenance.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedRecipe {
    id: RecipeId,
    display: RecipeDisplay,
    requirements: Option<Vec<RecipeIngredient>>,
    unlocked: Option<bool>,
    highlighted: Option<bool>,
    notification: Option<bool>,
    group: Option<String>,
    category: Option<String>,
}
impl ReceivedRecipe {
    /// Original opaque identity, not current placement permission.
    pub fn id(&self) -> &RecipeId {
        &self.id
    }
    /// Actual transmitted topology/display, not predicted inventory results.
    pub fn display(&self) -> &RecipeDisplay {
        &self.display
    }
    /// Received/native declared ingredient membership. Absence differs from empty.
    /// Membership alone does not prove a particular stack is recipe-book eligible.
    pub fn requirements(&self) -> Option<&[RecipeIngredient]> {
        self.requirements.as_deref()
    }
    /// Actual book membership, None until that membership has been established.
    pub fn unlocked(&self) -> Option<bool> {
        self.unlocked
    }
    /// Actual received book highlight, None if not established. Legacy add packets
    /// lack a separate highlight list, so a newly added entry stays unknown.
    pub fn highlighted(&self) -> Option<bool> {
        self.highlighted
    }
    /// Native modern toast flag; absent on legacy.
    pub fn notification(&self) -> Option<bool> {
        self.notification
    }
    /// Group supplied by this version: a legacy group string or modern numeric group.
    pub fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }
    /// Native category name, absent when not transmitted.
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }
    /// Possible displayed output item types, never exact stacks or crafting permission.
    pub fn output_items(&self, catalogue: &ReceivedRecipes) -> Result<Vec<RegistryId>> {
        if self.id.owner != catalogue.registries.stamp()
            || !catalogue.entries.iter().any(|r| r.id == self.id)
        {
            return Err(invalid(
                "recipe entry does not belong to this captured catalogue",
            ));
        }
        self.display
            .result()
            .ok_or_else(|| invalid("recipe has no transmitted result display"))?
            .item_ids(&catalogue.registries)
    }
}
/// Sealed recipes and actual registry/tag owner at one adapter capture boundary.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedRecipes {
    session: SessionStamp,
    receive_sequence: u64,
    book_initialized: bool,
    declaration_sequence: Option<u64>,
    book_sequence: Option<u64>,
    entries: Vec<ReceivedRecipe>,
    registries: Arc<ServerRegistryObservation>,
}
impl ReceivedRecipes {
    /// Owning world/transport at capture; recipe declarations may span respawns.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Applied packet boundary at capture, not freshness for every entry.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Whether an actual initial/reset book packet has established the catalogue.
    pub fn book_initialized(&self) -> bool {
        self.book_initialized
    }
    /// Legacy full-declaration ordinal. Modern does not transmit full named recipes.
    pub fn declaration_sequence(&self) -> Option<u64> {
        self.declaration_sequence
    }
    /// Last actual book mutation ordinal; settings alone are not membership.
    pub fn book_sequence(&self) -> Option<u64> {
        self.book_sequence
    }
    /// Entries actually supplied by this version, in stable native-key order.
    pub fn entries(&self) -> &[ReceivedRecipe] {
        &self.entries
    }
    /// Immutable registry/tag owner from the same capture boundary.
    pub fn registry_state(&self) -> &ServerRegistryObservation {
        &self.registries
    }
    /// Resolve a captured entry without accepting a reused native ID.
    pub fn entry(&self, id: &RecipeId) -> Result<&ReceivedRecipe> {
        self.entries
            .iter()
            .find(|r| r.id == *id)
            .ok_or_else(|| invalid("recipe identity is absent/replaced/foreign"))
    }
}
#[derive(Clone, Debug)]
pub(crate) struct RecipeEntry {
    pub(crate) display: RecipeDisplay,
    pub(crate) requirements: Option<Vec<RecipeIngredient>>,
    pub(crate) group: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) highlighted: Option<bool>,
    pub(crate) notification: Option<bool>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct RecipeReceipts {
    entries: BTreeMap<NativeRecipeId, (u64, RecipeEntry)>,
    declaration_sequence: Option<u64>,
    book_sequence: Option<u64>,
    book_initialized: bool,
    legacy_unlocked: BTreeSet<String>,
    legacy_highlighted: BTreeSet<String>,
    legacy_highlight_unknown: BTreeSet<String>,
}
impl RecipeReceipts {
    pub(crate) fn ghost_declaration(&self, name: &str) -> Option<(u64, RecipeDisplay)> {
        self.entries
            .get(&NativeRecipeId::Legacy(name.into()))
            .map(|(ordinal, entry)| (*ordinal, entry.display.clone()))
    }
    pub(crate) fn declare_legacy(
        &mut self,
        entries: BTreeMap<NativeRecipeId, RecipeEntry>,
        sequence: u64,
    ) {
        self.entries = entries
            .into_iter()
            .map(|(id, e)| (id, (sequence, e)))
            .collect();
        self.declaration_sequence = Some(sequence);
    }
    pub(crate) fn legacy_book(
        &mut self,
        unlocked: BTreeSet<String>,
        highlighted: BTreeSet<String>,
        initial: bool,
        sequence: u64,
    ) {
        if initial {
            self.legacy_highlight_unknown.clear();
        } else {
            self.legacy_highlight_unknown
                .retain(|id| unlocked.contains(id));
            self.legacy_highlight_unknown
                .extend(unlocked.difference(&self.legacy_unlocked).cloned());
        }
        self.legacy_unlocked = unlocked;
        self.legacy_highlighted = highlighted;
        self.book_initialized |= initial;
        self.book_sequence = Some(sequence);
    }
    pub(crate) fn add_modern(
        &mut self,
        entries: Vec<(i32, RecipeEntry)>,
        replace: bool,
        sequence: u64,
    ) {
        if replace {
            self.entries.clear();
            self.book_initialized = true;
        }
        for (id, e) in entries {
            self.entries
                .insert(NativeRecipeId::Modern(id), (sequence, e));
        }
        self.book_sequence = Some(sequence);
    }
    pub(crate) fn remove_modern(&mut self, ids: &[i32], sequence: u64) {
        for id in ids {
            self.entries.remove(&NativeRecipeId::Modern(*id));
        }
        self.book_sequence = Some(sequence);
    }
    pub(crate) fn capture(
        &self,
        session: SessionStamp,
        sequence: u64,
        registries: ServerRegistryObservation,
    ) -> Result<ReceivedRecipes> {
        if registries.session() != session || registries.receive_sequence() != sequence {
            return Err(invalid("recipe/registry capture boundaries differ"));
        }
        let owner = Arc::new(registries);
        let mut entries = Vec::new();
        for (native, (ordinal, e)) in &self.entries {
            if *ordinal > sequence || *ordinal < owner.stamp().configuration_generation {
                return Err(invalid("recipe declaration outside captured configuration"));
            }
            let mut display = e.display.clone();
            display.bind(&owner)?;
            let (unlocked, highlighted) = match native {
                NativeRecipeId::Modern(_) => (Some(true), e.highlighted),
                NativeRecipeId::Legacy(name) => (
                    if self.legacy_unlocked.contains(name) {
                        Some(true)
                    } else if self.book_initialized {
                        Some(false)
                    } else {
                        None
                    },
                    if self.book_initialized && !self.legacy_highlight_unknown.contains(name) {
                        Some(self.legacy_highlighted.contains(name))
                    } else {
                        None
                    },
                ),
            };
            entries.push(ReceivedRecipe {
                id: RecipeId {
                    owner: owner.stamp(),
                    entry_sequence: *ordinal,
                    native: native.clone(),
                },
                display,
                requirements: e.requirements.clone(),
                unlocked,
                highlighted,
                notification: e.notification,
                group: e.group.clone(),
                category: e.category.clone(),
            });
        }
        Ok(ReceivedRecipes {
            session,
            receive_sequence: sequence,
            book_initialized: self.book_initialized,
            declaration_sequence: self.declaration_sequence,
            book_sequence: self.book_sequence,
            entries,
            registries: owner,
        })
    }
}

pub(crate) fn legacy_entries(
    recipes: &crate::versions::java_1_16_1::server_registry::ServerRecipes,
) -> Result<BTreeMap<NativeRecipeId, RecipeEntry>> {
    use crate::versions::java_1_16_1::server_registry::ServerRecipeData as D;
    fn stack(v: &crate::versions::java_1_16_1::inventory::ItemStack) -> Result<RecipeSlotDisplay> {
        match super::super::legacy_slot(Some(v))? {
            SlotKnowledge::Item { item } => Ok(RecipeSlotDisplay::Stack { item }),
            SlotKnowledge::Empty => Ok(RecipeSlotDisplay::Empty),
            SlotKnowledge::Unavailable => Err(invalid("legacy recipe stack unavailable")),
        }
    }
    fn ingredient(
        v: &crate::versions::java_1_16_1::server_registry::Ingredient,
    ) -> Result<(RecipeSlotDisplay, RecipeIngredient)> {
        let displays = v.iter().flatten().map(stack).collect::<Result<Vec<_>>>()?;
        let items = displays
            .iter()
            .filter_map(|v| match v {
                RecipeSlotDisplay::Stack { item } => Some(item.id),
                _ => None,
            })
            .collect();
        Ok((
            if displays.is_empty() {
                RecipeSlotDisplay::Empty
            } else {
                RecipeSlotDisplay::Composite { contents: displays }
            },
            RecipeIngredient::Items { items },
        ))
    }
    let mut out = BTreeMap::new();
    for (name, r) in &recipes.recipes {
        let mut group = None;
        let mut requirements = None;
        let display = match &r.data {
            D::Shaped {
                width,
                height,
                group: g,
                ingredients,
                result,
            } => {
                group = Some(g.clone());
                let pairs = ingredients
                    .iter()
                    .map(ingredient)
                    .collect::<Result<Vec<_>>>()?;
                requirements = Some(
                    pairs
                        .iter()
                        .filter(|(d, _)| !matches!(d, RecipeSlotDisplay::Empty))
                        .map(|(_, r)| r.clone())
                        .collect(),
                );
                RecipeDisplay::Shaped {
                    width: *width,
                    height: *height,
                    ingredients: pairs.into_iter().map(|(d, _)| d).collect(),
                    result: stack(result)?,
                    crafting_station: None,
                }
            }
            D::Shapeless {
                group: g,
                ingredients,
                result,
            } => {
                group = Some(g.clone());
                let pairs = ingredients
                    .iter()
                    .map(ingredient)
                    .collect::<Result<Vec<_>>>()?;
                requirements = Some(pairs.iter().map(|(_, r)| r.clone()).collect());
                RecipeDisplay::Shapeless {
                    ingredients: pairs.into_iter().map(|(d, _)| d).collect(),
                    result: stack(result)?,
                    crafting_station: None,
                }
            }
            D::Cooking {
                group: g,
                ingredient: i,
                result,
                experience,
                cook_time,
            } => {
                group = Some(g.clone());
                RecipeDisplay::Furnace {
                    recipe_kind: r.kind.clone(),
                    ingredient: ingredient(i)?.0,
                    fuel: None,
                    result: stack(result)?,
                    crafting_station: None,
                    duration: *cook_time,
                    experience_bits: experience.to_bits(),
                }
            }
            D::Stonecutting {
                group: g,
                ingredient: i,
                result,
            } => {
                group = Some(g.clone());
                RecipeDisplay::Stonecutter {
                    input: ingredient(i)?.0,
                    result: stack(result)?,
                    crafting_station: None,
                }
            }
            D::Smithing {
                base,
                addition,
                result,
            } => RecipeDisplay::Smithing {
                template: None,
                base: ingredient(base)?.0,
                addition: ingredient(addition)?.0,
                result: stack(result)?,
                crafting_station: None,
            },
            D::Special => RecipeDisplay::Special {
                recipe_kind: r.kind.clone(),
            },
        };
        out.insert(
            NativeRecipeId::Legacy(name.clone()),
            RecipeEntry {
                display,
                requirements,
                group,
                category: None,
                highlighted: None,
                notification: None,
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
