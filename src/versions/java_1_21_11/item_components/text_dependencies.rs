//! Persistent text item fields, distinct from the item stream constructor.
//! Unimplemented persistent component constructors remain explicit errors.
use crate::{
    MinecraftVersion,
    client::{
        ItemComponent, ItemComponentPatch, ItemData, ItemStack, nbt::NbtValue, registry::Registry,
    },
};
use anyhow::{Context, Result, bail};

pub(super) fn identifier(value: &NbtValue) -> Result<String> {
    let NbtValue::String(value) = value else {
        bail!("native dependency identifier must be a string")
    };
    let spelling = value.text()?;
    let (namespace, path) = crate::client::identifier::parts(&spelling)?;
    Ok(format!("{namespace}:{path}"))
}
pub(super) fn hover_item(value: &NbtValue) -> Result<ItemStack> {
    let fields = value
        .as_compound()
        .context("native item hover must be a compound")?;
    let registry = Registry::for_version(MinecraftVersion::Java1_21_11);
    let item = registry.item(&identifier(
        fields.get("id").context("native hover item id required")?,
    )?)?;
    if item.name == "minecraft:air" {
        bail!("native hover item must not be air")
    }
    // Original lenientOptionalFieldOf count accepts Number.intValue in1..99;
    // bad types or out-of-range values use1, rather than dropping the event.
    let count = fields
        .get("count")
        .and_then(super::text::integer)
        .filter(|v| (1..=99).contains(v))
        .unwrap_or(1) as u32;
    let mut patch = ItemComponentPatch {
        added: Vec::new(),
        removed: Vec::new(),
    };
    if let Some(value) = fields.get("components") {
        let fields = value
            .as_compound()
            .context("native persistent component patch must be a compound")?;
        for entry in fields.entries() {
            let spelling = entry.key().text()?;
            let (removed, spelling) = match spelling.strip_prefix('!') {
                Some(v) => (true, v),
                None => (false, spelling.as_str()),
            };
            let (namespace, path) = crate::client::identifier::parts(spelling)?;
            let component = registry.item_component(&format!("{namespace}:{path}"))?;
            if removed {
                if !entry
                    .value()
                    .as_compound()
                    .is_some_and(|v| v.entries().is_empty())
                {
                    return Err(crate::client::constructor::Unresolved(
                        "unresolved native persistent removal value",
                    )
                    .into());
                }
                patch.removed.push(component);
                continue;
            }
            let mut bytes = Vec::new();
            match component.name.as_str() {
                "minecraft:max_stack_size" | "minecraft:damage" => {
                    let value = super::text::integer(entry.value())
                        .context("native persistent component integer required")?;
                    if component.name == "minecraft:max_stack_size" && !(1..=99).contains(&value) {
                        bail!("native persistent max stack size outside1..99")
                    }
                    if component.name == "minecraft:damage" && value < 0 {
                        bail!("native persistent damage must be nonnegative")
                    }
                    crate::protocol::put_varint(&mut bytes, value);
                }
                _ => {
                    return Err(crate::client::constructor::Unresolved(
                        "unresolved native persistent hover item component constructor",
                    )
                    .into());
                }
            }
            patch.added.push(ItemComponent {
                definition: component,
                bytes,
            });
        }
    }
    super::validate_patch(&patch).map_err(|_| {
        crate::client::constructor::Unresolved("unresolved native persistent patch validation")
    })?;
    Ok(ItemStack {
        id: item.id,
        name: item.name,
        count,
        data: if patch.added.is_empty() && patch.removed.is_empty() {
            ItemData::Default
        } else {
            ItemData::ModernComponents { patch }
        },
    })
}
