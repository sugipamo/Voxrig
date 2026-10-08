//! Registry-bound typed comparison fields, with explicit unresolved constructors.
use super::{definition, framing, values::Value};
use crate::client::{
    item_semantics::{Component as Key, Item, ModernNbt},
    registry::ServerRegistryObservation,
};
use anyhow::{Context as _, Result, bail};
use std::collections::BTreeMap;

pub(super) struct Context<'a> {
    pub registries: &'a ServerRegistryObservation,
    remaining: usize,
}
impl<'a> Context<'a> {
    pub(super) fn new(registries: &'a ServerRegistryObservation) -> Result<Self> {
        if registries.stamp().version != crate::MinecraftVersion::Java1_21_11
            || !registries.complete()
        {
            bail!("component comparison requires a complete owning modern configuration");
        }
        Ok(Self {
            registries,
            remaining: 65_536,
        })
    }
    pub(super) fn component(&mut self, value: &Value, depth: usize) -> Result<Key> {
        if depth > 256 {
            bail!("component comparison depth limit");
        }
        self.remaining = self
            .remaining
            .checked_sub(1)
            .context("component comparison work limit")?;
        let next = depth + 1;
        Ok(match value {
            Value::Unit => Key::Unit,
            Value::Boolean(v) => Key::Boolean(*v),
            Value::Integer(v) => Key::Integer(*v),
            Value::Float { bits } => Key::Float(if f32::from_bits(*bits).is_nan() {
                f32::NAN.to_bits()
            } else {
                *bits
            }),
            Value::Double { bits } => Key::Double(if f64::from_bits(*bits).is_nan() {
                f64::NAN.to_bits()
            } else {
                *bits
            }),
            Value::BlockPosition { packed } => Key::BlockPosition(*packed),
            Value::Uuid { most, least } => Key::Uuid(*most, *least),
            Value::String(v) => Key::String(v.clone()),
            Value::Identifier { namespace, path } => {
                Key::Identifier(namespace.clone(), path.clone())
            }
            Value::Forward { codec, value } => {
                Key::Forward(*codec, Box::new(self.component(value, next)?))
            }
            Value::Enumeration { codec, native_id } => Key::Enumeration(*codec, *native_id),
            Value::Nbt(v) => Key::Nbt(Some(ModernNbt(
                v.clone().context("unresolved nullable NBT constructor")?,
            ))),
            Value::Text {
                fields, field_key, ..
            } => Key::Text(match field_key {
                Some(key) => key.clone(),
                None => Box::new(
                    self.resolve_text(fields, next)?
                        .modern_field_key()
                        .context("unresolved native text dependency")?,
                ),
            }),
            Value::Profile(v) => Key::Profile(v.clone()),
            Value::Enchantments(v) => {
                let mut levels = BTreeMap::new();
                for (id, level) in &v.levels {
                    levels.insert(
                        self.registries
                            .bind_entry("minecraft:enchantment", *id)?
                            .into(),
                        *level,
                    );
                }
                Key::Enchantments(levels)
            }
            Value::Enchantability(v) => Key::Enchantability(v.clone()),
            Value::WritableBook(v) => Key::WritableBook(v.clone()),
            Value::WrittenBook {
                fields, field_key, ..
            } => Key::WrittenBook(match field_key {
                Some(key) => key.clone(),
                None => {
                    let mut book = fields.clone();
                    for page in &mut book.pages {
                        *page.raw = self.resolve_text(&page.raw, next)?;
                        if let Some(text) = &page.filtered {
                            page.filtered = Some(Box::new(self.resolve_text(text, next)?));
                        }
                    }
                    Box::new(
                        book.field_comparison()
                            .context("unresolved native book dependency")?,
                    )
                }
            }),
            Value::Sequence(values) => Key::Sequence(self.list(values, next)?),
            Value::List(values) => Key::List(self.list(values, next)?),
            Value::Map(values) => {
                let mut map = BTreeMap::new();
                for (key, value) in values {
                    let key = self.component(key, next)?.map_key()?;
                    let value = self.component(value, next)?;
                    map.insert(key, value);
                }
                Key::Map(map)
            }
            Value::Optional(value) => Key::Optional(
                value
                    .as_ref()
                    .map(|v| self.component(v, next).map(Box::new))
                    .transpose()?,
            ),
            Value::Either { left, value } => {
                Key::Either(*left, Box::new(self.component(value, next)?))
            }
            Value::Registry {
                registry,
                native_id,
            } => Key::Registry(self.registries.bind_entry(registry, *native_id)?.into()),
            Value::HolderReference {
                registry,
                native_id,
            } => Key::Reference(self.registries.bind_entry(registry, *native_id)?.into()),
            Value::HolderInline { registry, value } => {
                Key::Inline(registry.clone(), Box::new(self.component(value, next)?))
            }
            Value::HolderTag { registry, tag } => {
                let tags = self
                    .registries
                    .tags()
                    .context("holder tag declaration was not received")?;
                if !tags
                    .value
                    .get(registry)
                    .is_some_and(|tags| tags.contains_key(tag))
                {
                    bail!("holder tag was not declared by this registry configuration");
                }
                let owner = self.registries.stamp();
                Key::Tag {
                    registry: registry.clone(),
                    name: tag.clone(),
                    owner: (owner.connection_id, owner.configuration_generation),
                }
            }
            Value::HolderList { registry, values } => {
                Key::HolderList(registry.clone(), self.list(values, next)?)
            }
            Value::ProfileProperties(values) => {
                let mut properties = BTreeMap::<String, Vec<(String, Option<String>)>>::new();
                for v in values {
                    properties
                        .entry(v.name.clone())
                        .or_default()
                        .push((v.value.clone(), v.signature.clone()));
                }
                Key::ProfileProperties(properties)
            }
            Value::TypedComponent { native_id, value } => {
                definition(*native_id)?;
                Key::Typed(*native_id, Box::new(self.component(value, next)?))
            }
            Value::Item(value) => Key::Item(Box::new(self.item(value, next)?)),
            Value::Bundle(value) => {
                let items = value
                    .items
                    .iter()
                    .map(|v| self.item(v, next))
                    .collect::<Result<_>>()?;
                Key::Bundle(items, value.weight)
            }
            Value::Patch { added, removed } => Key::Patch(self.patch(added, removed, next)?),
            Value::Dispatch { left, tag, value } => {
                Key::Dispatch(*left, *tag, Box::new(self.component(value, next)?))
            }
        })
    }
    fn list(&mut self, values: &[Value], depth: usize) -> Result<Vec<Key>> {
        values.iter().map(|v| self.component(v, depth)).collect()
    }
    fn patch(
        &mut self,
        added: &[(i32, Value)],
        removed: &[i32],
        depth: usize,
    ) -> Result<BTreeMap<i32, Option<Key>>> {
        let mut patch = BTreeMap::new();
        for (id, v) in added {
            definition(*id)?;
            if patch.insert(*id, Some(self.component(v, depth)?)).is_some() {
                bail!("duplicate native component patch type");
            }
        }
        for id in removed {
            definition(*id)?;
            if patch.insert(*id, None).is_some() {
                bail!("duplicate native component patch type");
            }
        }
        Ok(patch)
    }
    fn item(
        &mut self,
        item: &crate::client::item_constructor::Item<Box<Value>>,
        depth: usize,
    ) -> Result<Item> {
        let Some(id) = item.native_id else {
            return Ok(Item::empty());
        };
        let mut components = BTreeMap::new();
        for field in crate::client::modern_prototype_components(id.value())? {
            let value =
                framing::decode_value(definition(field.definition.id.value())?, &field.bytes)?;
            components.insert(field.definition.id.value(), self.component(&value, depth)?);
        }
        if let Some(patch) = &item.patch {
            let Value::Patch { added, removed } = patch.as_ref() else {
                bail!("nested item patch has another native constructor");
            };
            for (id, value) in self.patch(added, removed, depth)? {
                if let Some(value) = value {
                    components.insert(id, value);
                } else {
                    components.remove(&id);
                }
            }
        }
        Ok(Item {
            count: item.count,
            native_id: Some(id),
            components,
        })
    }
    fn stack_fields(&mut self, item: &crate::client::ItemStack, depth: usize) -> Result<Item> {
        use crate::client::{ItemData, item_components::ComponentFields};
        let definition =
            crate::client::registry::Registry::for_version(crate::MinecraftVersion::Java1_21_11)
                .item_by_native_id(item.id.value())?;
        if definition.id != item.id || definition.name != item.name {
            bail!("item identity belongs to another version or type");
        }
        let patch = match &item.data {
            ItemData::Default => None,
            ItemData::ModernComponents { patch } => {
                super::validate_patch(patch)?;
                Some(patch)
            }
            _ => bail!("item data belongs to another version"),
        };
        let fields = ComponentFields::apply(
            crate::MinecraftVersion::Java1_21_11,
            crate::client::modern_prototype_components(item.id.value())?,
            patch,
        )?;
        let mut components = BTreeMap::new();
        for field in fields.iter() {
            let value = framing::decode_value(
                super::definition(field.definition.id.value())?,
                &field.bytes,
            )?;
            components.insert(field.definition.id.value(), self.component(&value, depth)?);
        }
        Ok(Item {
            count: i32::try_from(item.count)?,
            native_id: Some(item.id),
            components,
        })
    }
    fn resolve_text(
        &mut self,
        value: &crate::client::text::Text,
        depth: usize,
    ) -> Result<crate::client::text::Text> {
        if let Some(source) = &value.source {
            return super::text::project_bound(
                source,
                &mut TextDependencies {
                    context: self,
                    base_depth: depth,
                },
            );
        }
        use crate::client::text::{Argument, Click, Contents, Hover};
        if depth > 256 {
            bail!("item text dependency depth limit")
        }
        self.remaining = self
            .remaining
            .checked_sub(1)
            .context("item text dependency work limit")?;
        let next = depth + 1;
        let mut value = value.clone();
        match &mut value.contents {
            Contents::Translate { arguments, .. } => {
                for argument in arguments {
                    if let Argument::Text(text) = argument {
                        **text = self.resolve_text(text, next)?;
                    }
                }
            }
            Contents::Selector { separator, .. } | Contents::Nbt { separator, .. } => {
                if let Some(text) = separator {
                    **text = self.resolve_text(text, next)?;
                }
            }
            _ => {}
        }
        if let Some(hover) = &mut value.style.hover {
            match hover {
                Hover::Item(raw) => {
                    let item = super::text_dependencies::hover_item(raw)?;
                    *hover = Hover::BoundItem(Box::new(self.stack_fields(&item, next)?));
                }
                Hover::Text(text) => **text = self.resolve_text(text, next)?,
                Hover::Entity(entity) => {
                    if let Some(text) = &mut entity.name {
                        **text = self.resolve_text(text, next)?;
                    }
                }
                Hover::BoundItem(_) => {}
            }
        }
        if let Some(Click::Dialog(raw)) = &value.style.click {
            let name = super::text_dependencies::identifier(raw)
                .context("unresolved native inline dialog constructor")?;
            let entry = self.registries.find_entry("minecraft:dialog", &name)?;
            value.style.click = Some(Click::BoundDialog(entry.into()));
        }
        for text in &mut value.siblings {
            *text = self.resolve_text(text, next)?;
        }
        Ok(value)
    }
}

#[cfg(test)]
pub(super) fn field(
    value: &crate::client::ItemComponent,
    registries: &ServerRegistryObservation,
) -> Result<Key> {
    let native = definition(value.definition.id.value())?;
    if value.definition.id.version() != crate::MinecraftVersion::Java1_21_11
        || value.definition.name != native.name
    {
        bail!("component identity belongs to another version or type");
    }
    Context::new(registries)?.component(&framing::decode_value(native, &value.bytes)?, 0)
}

pub(crate) fn stack(
    item: &crate::client::ItemStack,
    registries: &ServerRegistryObservation,
) -> Result<Item> {
    Context::new(registries)?.stack_fields(item, 0)
}

struct TextDependencies<'a, 'r> {
    context: &'a mut Context<'r>,
    base_depth: usize,
}
impl super::text::Resolver for TextDependencies<'_, '_> {
    fn charge(&mut self) -> Result<()> {
        self.context.remaining = self
            .context
            .remaining
            .checked_sub(1)
            .ok_or(crate::client::constructor::Limit("bound text work limit"))?;
        Ok(())
    }
    fn item(
        &mut self,
        value: &crate::client::nbt::NbtValue,
        depth: usize,
    ) -> Result<crate::client::text::Hover> {
        let item = super::text_dependencies::hover_item(value)?;
        let value = self
            .context
            .stack_fields(&item, self.base_depth + depth + 1)
            .map_err(|error| {
                if crate::client::constructor::preserve(&error) {
                    error
                } else {
                    anyhow::Error::from(crate::client::constructor::Unresolved(
                        "bound hover item component context is unresolved",
                    ))
                    .context(error)
                }
            })?;
        Ok(crate::client::text::Hover::BoundItem(Box::new(value)))
    }
    fn dialog(
        &mut self,
        value: &crate::client::nbt::NbtValue,
        _depth: usize,
    ) -> Result<crate::client::text::Click> {
        if !matches!(
            value,
            crate::client::nbt::NbtValue::String(_) | crate::client::nbt::NbtValue::Compound(_)
        ) {
            bail!("native dialog must be a reference string or inline compound");
        }
        if !matches!(value, crate::client::nbt::NbtValue::String(_)) {
            return Err(crate::client::constructor::Unresolved(
                "unresolved native inline dialog constructor",
            )
            .into());
        }
        let name = super::text_dependencies::identifier(value)?;
        if !self
            .context
            .registries
            .registries()
            .contains_key("minecraft:dialog")
        {
            return Err(
                crate::client::constructor::Unresolved("dialog registry was not received").into(),
            );
        }
        Ok(crate::client::text::Click::BoundDialog(
            self.context
                .registries
                .find_entry("minecraft:dialog", &name)?
                .into(),
        ))
    }
}
