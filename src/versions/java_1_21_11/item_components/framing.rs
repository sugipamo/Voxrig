//! Original codec composition describes boundaries, not decoded gameplay meaning.
//! Registry references remain encoded references until a received registry binding
//! and semantic interpretation are established. Non-default operations stay blocked.
use super::{Definition, Reader, definition, definitions};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
struct Schema {
    roots: BTreeMap<String, usize>,
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    native_class: String,
    #[serde(flatten)]
    rule: Rule,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Rule {
    Sequence {
        children: Vec<usize>,
    },
    Forward {
        child: usize,
    },
    Boolean,
    Fixed {
        length: usize,
    },
    Varint,
    Nbt,
    Unit,
    String {
        maximum: usize,
    },
    Optional {
        child: usize,
    },
    List {
        child: usize,
        maximum: usize,
    },
    Map {
        key: usize,
        value: usize,
        maximum: usize,
    },
    Either {
        left: usize,
        right: usize,
    },
    Registry {
        registry: String,
    },
    Holder {
        registry: String,
        inline: usize,
    },
    HolderSet {
        registry: String,
        child: usize,
    },
    ProfileProperties,
    TypedComponent,
    Item {
        nonempty: bool,
    },
    Patch,
    Dispatch {
        branched: bool,
        variants: Vec<Variant>,
    },
}
#[derive(Deserialize)]
struct Variant {
    tag: i32,
    left: Option<bool>,
    codec: usize,
}
fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../data/client_api/item_component_schema-1.21.11.json"
        ))
        .expect("valid pinned original item-component composition")
    })
}

pub(super) struct Budget {
    initial_bytes: usize,
    steps: usize,
}
impl Budget {
    pub(super) fn new(r: &Reader<'_>) -> Self {
        Self {
            initial_bytes: r.remaining().len(),
            steps: 65_536,
        }
    }
    fn check(&self, r: &Reader<'_>) -> Result<()> {
        if self.initial_bytes.saturating_sub(r.remaining().len()) > 1_048_576 {
            bail!("item-component patch byte limit");
        }
        Ok(())
    }
    fn enter(&mut self, r: &Reader<'_>, depth: usize) -> Result<()> {
        if depth > 256 {
            bail!("item-component codec depth limit");
        }
        self.steps = self
            .steps
            .checked_sub(1)
            .context("item-component work limit")?;
        self.check(r)
    }
}
pub(super) fn value(
    r: &mut Reader<'_>,
    native: &Definition,
    budget: &mut Budget,
    depth: usize,
) -> Result<()> {
    read_value(r, root(native)?, budget, depth, false)?;
    Ok(())
}
fn root(native: &Definition) -> Result<usize> {
    let root = *schema()
        .roots
        .get(&native.name)
        .context("missing pinned component codec")?;
    if schema().nodes[root].native_class != native.stream_codec_class {
        bail!("pinned component codec composition differs from registry");
    }
    Ok(root)
}
pub(super) fn decode_value(native: &Definition, bytes: &[u8]) -> Result<super::values::Value> {
    let mut reader = Reader::new(bytes);
    let mut budget = Budget::new(&reader);
    let value = read_value(&mut reader, root(native)?, &mut budget, 0, true)?;
    reader.end()?;
    Ok(value)
}
fn reference(r: &mut Reader<'_>, registry: &str) -> Result<i32> {
    let id = r.varint()?;
    if id < 0 || !registry.starts_with("minecraft:") {
        bail!("invalid encoded registry reference: {registry}/{id}");
    }
    Ok(id)
}
fn text(r: &mut Reader<'_>, maximum: usize) -> Result<String> {
    let value = r.string()?;
    if value.encode_utf16().count() > maximum {
        bail!("native item-component string length limit");
    }
    Ok(value)
}
fn nested_patch(
    r: &mut Reader<'_>,
    budget: &mut Budget,
    depth: usize,
    capture: bool,
    mut weight: Option<&mut crate::client::item_constructor::WeightFields>,
) -> Result<super::values::Value> {
    use super::values::Value;
    let added_count = r.count(definitions().len())?;
    let removed_count = r.count(definitions().len())?;
    if added_count + removed_count > definitions().len() {
        bail!("nested item-component patch type count limit");
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for _ in 0..added_count {
        let id = r.varint()?;
        let native = definition(id)?;
        if !seen.insert(id) {
            bail!("duplicate nested item-component type");
        }
        let value = if let Some(fields) = weight.as_deref_mut() {
            match native.name.as_str() {
                "minecraft:max_stack_size" => {
                    let value = read_value(r, root(native)?, budget, depth + 1, true)?;
                    let Value::Integer(size) = &value else {
                        bail!("native stack size integer required");
                    };
                    fields.max_stack_size = *size;
                    value
                }
                "minecraft:bundle_contents" => {
                    if root(native)? != 513 {
                        bail!("native bundle root changed; update Voxrig");
                    }
                    budget.enter(r, depth + 1)?;
                    let (value, contents) = read_bundle(r, budget, depth + 1, capture)?;
                    fields.bundle = Some(contents);
                    value
                }
                "minecraft:bees" => {
                    if root(native)? != 614 {
                        bail!("native bee root changed; update Voxrig");
                    }
                    budget.enter(r, depth + 1)?;
                    let (value, present) = read_bees(r, budget, depth + 1, capture)?;
                    fields.has_bees = present;
                    value
                }
                _ => read_value(r, root(native)?, budget, depth + 1, capture)?,
            }
        } else {
            read_value(r, root(native)?, budget, depth + 1, capture)?
        };
        if capture {
            added.push((id, value));
        }
    }
    for _ in 0..removed_count {
        let id = r.varint()?;
        definition(id)?;
        if !seen.insert(id) {
            bail!("duplicate nested item-component type");
        }
        if let Some(fields) = weight.as_deref_mut() {
            match definition(id)?.name.as_str() {
                "minecraft:max_stack_size" => fields.max_stack_size = 1,
                "minecraft:bundle_contents" => fields.bundle = None,
                "minecraft:bees" => fields.has_bees = false,
                _ => {}
            }
        }
        if capture {
            removed.push(id);
        }
    }
    budget.check(r)?;
    Ok(if capture {
        Value::Patch { added, removed }
    } else {
        Value::Unit
    })
}

fn read_item(
    r: &mut Reader<'_>,
    budget: &mut Budget,
    depth: usize,
    capture: bool,
    nonempty: bool,
    weight_needed: bool,
) -> Result<(
    super::values::Value,
    Option<crate::client::item_constructor::WeightFields>,
)> {
    use super::values::Value;
    use crate::client::item_constructor::Item;
    let count = r.varint()?;
    if count <= 0 {
        if nonempty {
            bail!("empty nested native item is not allowed");
        }
        return Ok((
            if capture {
                Value::Item(Item::empty())
            } else {
                Value::Unit
            },
            None,
        ));
    }
    let id = r.varint()?;
    let definition =
        crate::client::registry::Registry::for_version(crate::MinecraftVersion::Java1_21_11)
            .item_by_native_id(id)?;
    if nonempty && id == 0 {
        bail!("empty air stack in nonempty nested item codec");
    }
    let mut weight = if weight_needed {
        Some(crate::client::modern_weight_defaults(id)?)
    } else {
        None
    };
    let patch = nested_patch(r, budget, depth + 1, capture, weight.as_mut())?;
    let fields = if id == 0 {
        Item::empty()
    } else {
        Item {
            count,
            native_id: Some(definition.id),
            patch: if capture { Some(Box::new(patch)) } else { None },
        }
    };
    Ok((
        if capture {
            Value::Item(fields)
        } else {
            Value::Unit
        },
        weight,
    ))
}

// These helpers receive an already charged Forward root. On normal receipt,
// retain just constructor arithmetic inputs, never the complete patch/value tree.
fn read_bundle(
    r: &mut Reader<'_>,
    budget: &mut Budget,
    depth: usize,
    capture: bool,
) -> Result<(super::values::Value, crate::client::fraction::Fraction)> {
    use super::values::Value;
    use crate::client::{fraction::Fraction, item_constructor::Bundle};
    let source = schema();
    if source.nodes[513].native_class != "aao$14"
        || !matches!(source.nodes[513].rule, Rule::Forward { child: 514 })
    {
        bail!("native bundle constructor composition changed; update Voxrig");
    }
    let Rule::List {
        child: 454,
        maximum,
    } = source.nodes[514].rule
    else {
        bail!("native bundle list codec changed; update Voxrig");
    };
    if source.nodes[454].native_class != "dlt$2"
        || !matches!(source.nodes[454].rule, Rule::Item { nonempty: true })
    {
        bail!("native bundle item codec changed; update Voxrig");
    }
    budget.enter(r, depth + 1)?;
    let mut items = Vec::new();
    let mut total = Fraction::ZERO;
    for _ in 0..r.count(maximum.min(65_536))? {
        budget.enter(r, depth + 2)?;
        let before = r.remaining();
        let (item, fields) = read_item(r, budget, depth + 2, capture, true, true)?;
        // The native nonempty codec's positive count is the first VarInt.
        let count = Reader::new(before).varint()?;
        total = total.add(
            fields
                .context("missing native bundle weight fields")?
                .weight(count)?,
        )?;
        if capture {
            let Value::Item(item) = item else {
                bail!("native bundle item fields required");
            };
            items.push(item);
        }
    }
    budget.check(r)?;
    Ok((
        if capture {
            Value::Bundle(Box::new(Bundle {
                items,
                weight: total,
            }))
        } else {
            Value::Unit
        },
        total,
    ))
}
fn read_bees(
    r: &mut Reader<'_>,
    budget: &mut Budget,
    depth: usize,
    capture: bool,
) -> Result<(super::values::Value, bool)> {
    use super::values::Value;
    let source = schema();
    if source.nodes[614].native_class != "aao$14"
        || !matches!(source.nodes[614].rule, Rule::Forward { child: 615 })
    {
        bail!("native bee constructor composition changed; update Voxrig");
    }
    let Rule::List {
        child: 616,
        maximum,
    } = source.nodes[615].rule
    else {
        bail!("native bee list codec changed; update Voxrig");
    };
    budget.enter(r, depth + 1)?;
    let count = r.count(maximum.min(65_536))?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let value = read_value(r, 616, budget, depth + 2, capture)?;
        if capture {
            entries.push(value);
        }
    }
    budget.check(r)?;
    Ok((
        if capture {
            Value::Forward {
                codec: 614,
                value: Box::new(Value::List(entries)),
            }
        } else {
            Value::Unit
        },
        count != 0,
    ))
}
// Keep large text constructor temporaries out of each recursive grammar frame,
// including the normal receive path that never captures text.
#[inline(never)]
fn capture_text(
    tag: &std::sync::Arc<crate::client::nbt::NbtValue>,
) -> Result<super::values::Value> {
    let fields = super::text::project(tag)?;
    let dependencies = fields.dependencies();
    let field_key = fields.modern_field_key().map(Box::new);
    Ok(super::values::Value::Text {
        fields: Box::new(fields),
        dependencies,
        field_key,
    })
}

// Both enchantment roots use this same constructor. Keep only the effective
// reference/level map on the framing path, rather than allocating a Value tree.
fn read_enchantments(
    r: &mut Reader<'_>,
    child: usize,
    class: &str,
    budget: &mut Budget,
    depth: usize,
    capture: bool,
) -> Result<super::values::Value> {
    let Node {
        rule: Rule::Map {
            key,
            value,
            maximum,
        },
        ..
    } = &schema().nodes[child]
    else {
        bail!("native enchantment map codec changed; update Voxrig");
    };
    if class != "aao$17"
        || child != 19
        || *key != 20
        || *value != 2
        || !matches!(&schema().nodes[*key].rule, Rule::Registry { registry } if registry == "minecraft:enchantment")
        || !matches!(&schema().nodes[*value].rule, Rule::Varint)
    {
        bail!("native enchantment constructor composition changed; update Voxrig");
    }
    if depth + 1 > 256 {
        bail!("item-component codec depth limit");
    }
    budget.steps = budget
        .steps
        .checked_sub(1)
        .context("item-component work limit")?;
    let mut levels = BTreeMap::new();
    for _ in 0..r.count((*maximum).min(65_536))? {
        if depth + 2 > 256 {
            bail!("item-component codec depth limit");
        }
        budget.steps = budget
            .steps
            .checked_sub(2)
            .context("item-component work limit")?;
        let key = reference(r, "minecraft:enchantment")?;
        let level = r.varint()?;
        levels.insert(key, level);
    }
    let fields = crate::client::enchantments::Enchantments::from_entries(levels)?;
    Ok(if capture {
        super::values::Value::Enchantments(fields)
    } else {
        super::values::Value::Unit
    })
}

fn read_value(
    r: &mut Reader<'_>,
    node: usize,
    budget: &mut Budget,
    depth: usize,
    capture: bool,
) -> Result<super::values::Value> {
    use super::values::{self, ProfileProperty, Value};
    budget.enter(r, depth)?;
    let child_depth = depth + 1;
    let native = schema()
        .nodes
        .get(node)
        .context("invalid pinned codec node")?;
    let decoded = match &native.rule {
        Rule::Sequence { children } => {
            if node == 529 && (native.native_class != "aao$3" || children != &[530, 14, 2, 533, 5])
            {
                bail!("native written book constructor composition changed; update Voxrig");
            }
            let mut values = Vec::new();
            let mut generation = None;
            for (index, &child) in children.iter().enumerate() {
                // Even framing-only receipt must run the scalar constructor
                // check. Capture just that field, retaining no page Value tree.
                let generation_field = node == 529 && index == 2;
                let value = read_value(r, child, budget, child_depth, capture || generation_field)?;
                if generation_field {
                    let Value::Integer(value) = &value else {
                        bail!("native written book generation required");
                    };
                    generation = Some(*value);
                }
                if capture {
                    values.push(value);
                }
            }
            if let Some(generation) = generation {
                crate::client::books::validate_generation(generation)?;
            }
            let value = Value::Sequence(values);
            if capture && node == 583 {
                Value::Profile(super::profile::from_fields(&value)?)
            } else if capture && node == 529 {
                let fields = super::books::written(value)?;
                let dependencies = fields.dependencies();
                let field_key = fields.field_comparison().map(Box::new);
                Value::WrittenBook {
                    fields,
                    dependencies,
                    field_key,
                }
            } else {
                value
            }
        }
        Rule::Forward { child } => {
            if values::identifier_forward(node, *child, &native.native_class)? {
                // The native constructor always validates, including when the
                // caller only needs framing. Capture still avoids a value tree
                // on normal receipt; the existing string read owns the text.
                let Node {
                    rule: Rule::String { maximum },
                    ..
                } = schema()
                    .nodes
                    .get(*child)
                    .context("missing Identifier string codec")?
                else {
                    bail!("native Identifier child codec changed; update Voxrig");
                };
                budget.steps = budget
                    .steps
                    .checked_sub(1)
                    .context("item-component work limit")?;
                if child_depth > 256 {
                    bail!("item-component codec depth limit");
                }
                let spelling = text(r, *maximum)?;
                let (namespace, path) = crate::client::identifier::parts(&spelling)?;
                if capture {
                    Value::Identifier {
                        namespace: namespace.to_owned(),
                        path: path.to_owned(),
                    }
                } else {
                    Value::Unit
                }
            } else if node == 18 {
                read_enchantments(r, *child, &native.native_class, budget, depth, capture)?
            } else if node == 468 {
                if native.native_class != "aao$17" || *child != 2 {
                    bail!("native enchantability constructor composition changed; update Voxrig");
                }
                let Value::Integer(value) = read_value(r, *child, budget, child_depth, true)?
                else {
                    bail!("native enchantability integer required");
                };
                let fields = crate::client::books::Enchantability::new(value)?;
                if capture {
                    Value::Enchantability(fields)
                } else {
                    Value::Unit
                }
            } else if node == 513 {
                read_bundle(r, budget, depth, capture)?.0
            } else {
                let value = read_value(r, *child, budget, child_depth, capture)?;
                if capture && node == 7 {
                    let Value::Nbt(Some(tag)) = value else {
                        bail!("native text constructor requires a non-End NBT value");
                    };
                    capture_text(&tag)?
                } else if capture && node == 524 {
                    if native.native_class != "aao$14" || *child != 525 {
                        bail!(
                            "native writable book constructor composition changed; update Voxrig"
                        );
                    }
                    Value::WritableBook(super::books::writable(value)?)
                } else if capture {
                    Value::Forward {
                        codec: node,
                        value: Box::new(value),
                    }
                } else {
                    Value::Unit
                }
            }
        }
        Rule::Boolean => {
            // The original component boolean codec uses ByteBuf.readBoolean:
            // every nonzero byte is true, including skin-model bytes 2/255.
            let value = r.u8()? != 0;
            if capture {
                Value::Boolean(value)
            } else {
                Value::Unit
            }
        }
        Rule::Fixed { length } => {
            let bytes = r.take(*length)?;
            if capture {
                values::fixed(node, &native.native_class, bytes)?
            } else {
                Value::Unit
            }
        }
        Rule::Varint => {
            let value = r.varint()?;
            if !capture {
                Value::Unit
            } else if native.native_class == "aam$21" {
                Value::Enumeration {
                    codec: node,
                    native_id: values::enumeration(node, value)?,
                }
            } else {
                Value::Integer(value)
            }
        }
        Rule::Nbt => {
            if capture {
                Value::Nbt(
                    crate::client::nbt::decode_unnamed_tag(&r.encoded_nbt()?)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                )
            } else {
                r.skip_nbt()?;
                Value::Unit
            }
        }
        Rule::Unit => Value::Unit,
        Rule::String { maximum } => {
            let value = text(r, *maximum)?;
            if capture {
                Value::String(value)
            } else {
                Value::Unit
            }
        }
        Rule::Optional { child } => {
            let present = r.u8()? != 0;
            let value = if present {
                Some(read_value(r, *child, budget, child_depth, capture)?)
            } else {
                None
            };
            if capture {
                Value::Optional(value.map(Box::new))
            } else {
                Value::Unit
            }
        }
        Rule::List { child, maximum } => {
            let mut values = Vec::new();
            for _ in 0..r.count((*maximum).min(65_536))? {
                let value = read_value(r, *child, budget, child_depth, capture)?;
                if capture {
                    values.push(value);
                }
            }
            Value::List(values)
        }
        Rule::Map {
            key,
            value,
            maximum,
        } => {
            let mut entries = Vec::new();
            for _ in 0..r.count((*maximum).min(65_536))? {
                let key = read_value(r, *key, budget, child_depth, capture)?;
                let value = read_value(r, *value, budget, child_depth, capture)?;
                if capture {
                    entries.push((key, value));
                }
            }
            Value::Map(entries)
        }
        Rule::Either { left, right } => {
            let left_side = r.u8()? != 0;
            let child = if left_side { left } else { right };
            let value = read_value(r, *child, budget, child_depth, capture)?;
            if capture {
                Value::Either {
                    left: left_side,
                    value: Box::new(value),
                }
            } else {
                Value::Unit
            }
        }
        Rule::Registry { registry } => {
            let native_id = reference(r, registry)?;
            if capture {
                Value::Registry {
                    registry: registry.clone(),
                    native_id,
                }
            } else {
                Value::Unit
            }
        }
        Rule::Holder { registry, inline } => {
            let id = reference(r, registry)?;
            if id == 0 {
                let value = read_value(r, *inline, budget, child_depth, capture)?;
                if capture {
                    Value::HolderInline {
                        registry: registry.clone(),
                        value: Box::new(value),
                    }
                } else {
                    Value::Unit
                }
            } else if capture {
                Value::HolderReference {
                    registry: registry.clone(),
                    native_id: id - 1,
                }
            } else {
                Value::Unit
            }
        }
        Rule::HolderSet { registry, child } => {
            let count = reference(r, registry)?;
            if count == 0 {
                let tag = text(r, 32_767)?;
                if capture {
                    Value::HolderTag {
                        registry: registry.clone(),
                        tag,
                    }
                } else {
                    Value::Unit
                }
            } else {
                let count = usize::try_from(count - 1)?;
                if count > 65_536 {
                    bail!("item-component holder-set count limit");
                }
                let mut values = Vec::new();
                for _ in 0..count {
                    let value = read_value(r, *child, budget, child_depth, capture)?;
                    if capture {
                        values.push(value);
                    }
                }
                if capture {
                    Value::HolderList {
                        registry: registry.clone(),
                        values,
                    }
                } else {
                    Value::Unit
                }
            }
        }
        Rule::ProfileProperties => {
            let mut values = Vec::new();
            for _ in 0..r.count(65_536)? {
                let name = text(r, 64)?;
                let value = text(r, 32_767)?;
                let signature = if r.u8()? != 0 {
                    Some(text(r, 1024)?)
                } else {
                    None
                };
                if capture {
                    values.push(ProfileProperty {
                        name,
                        value,
                        signature,
                    });
                }
            }
            Value::ProfileProperties(values)
        }
        Rule::TypedComponent => {
            let native_id = r.varint()?;
            let native = definition(native_id)?;
            let value = read_value(r, root(native)?, budget, child_depth, capture)?;
            if capture {
                Value::TypedComponent {
                    native_id,
                    value: Box::new(value),
                }
            } else {
                Value::Unit
            }
        }
        Rule::Item { nonempty } => read_item(r, budget, depth, capture, *nonempty, false)?.0,
        Rule::Patch => nested_patch(r, budget, child_depth, capture, None)?,
        Rule::Dispatch { branched, variants } => {
            let left = if *branched { Some(r.bool()?) } else { None };
            let tag = r.varint()?;
            let chosen = variants
                .iter()
                .find(|v| v.tag == tag && v.left == left)
                .context("unknown native component dispatcher tag; update Voxrig")?;
            let value = read_value(r, chosen.codec, budget, child_depth, capture)?;
            if capture {
                Value::Dispatch {
                    left,
                    tag,
                    value: Box::new(value),
                }
            } else {
                Value::Unit
            }
        }
    };
    budget.check(r)?;
    Ok(if capture { decoded } else { Value::Unit })
}

#[cfg(test)]
fn read(r: &mut Reader<'_>, node: usize, budget: &mut Budget, depth: usize) -> Result<()> {
    read_value(r, node, budget, depth, false)?;
    Ok(())
}
#[cfg(test)]
pub(super) fn decode_node(node: usize, bytes: &[u8]) -> Result<super::values::Value> {
    let mut reader = Reader::new(bytes);
    let mut budget = Budget::new(&reader);
    let value = read_value(&mut reader, node, &mut budget, 0, true)?;
    reader.end()?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    #[test]
    fn original_composition_dispatchers_and_sources_match_the_original_registry() {
        let source: Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/item_component_schema_source.json"
        ))
        .unwrap();
        let component_source: Value = serde_json::from_str(include_str!(
            "../../../../data/client_api/item_component_source.json"
        ))
        .unwrap();
        for key in [
            "original_server_jar_sha1",
            "mappings_sha256",
            "original_classpath_entries_sha256",
        ] {
            assert_eq!(source[key], component_source[key]);
        }
        assert_eq!((schema().roots.len(), schema().nodes.len()), (105, 651));
        for native in definitions() {
            assert_eq!(
                schema().nodes[schema().roots[&native.name]].native_class,
                native.stream_codec_class
            );
        }
        let path = "data/client_api/item_component_schema-1.21.11.json";
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../../data/client_api/item_component_schema-1.21.11.json"
                ))
            ),
            source["files_sha256"][path]
        );
        for (path, bytes) in [
            (
                "scripts/ExportItemComponentSchema.java",
                include_bytes!("../../../../scripts/ExportItemComponentSchema.java").as_slice(),
            ),
            (
                "scripts/export_item_component_schema.py",
                include_bytes!("../../../../scripts/export_item_component_schema.py").as_slice(),
            ),
            (
                "scripts/ExportInventoryTransfers.java",
                include_bytes!("../../../../scripts/ExportInventoryTransfers.java").as_slice(),
            ),
            (
                "scripts/export_item_components.py",
                include_bytes!("../../../../scripts/export_item_components.py").as_slice(),
            ),
            (
                "scripts/export_regular_clicks.py",
                include_bytes!("../../../../scripts/export_regular_clicks.py").as_slice(),
            ),
        ] {
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                source["generators_sha256"][path]
            );
        }
        let sizes: Vec<_> = schema()
            .nodes
            .iter()
            .filter_map(|n| {
                if let Rule::Dispatch { variants, .. } = &n.rule {
                    Some(variants.len())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(sizes, vec![118, 3, 5]);
    }
    #[test]
    fn malformed_recursive_items_and_unknown_dispatchers_never_fabricate_a_value() {
        for bytes in [
            vec![1, 0, 48, 1, 0],
            vec![1, 0, 48, 1, 1, 0, 0, 0],
            vec![1, 0, 48, 1, 1, 1, 1, 0, 104],
        ] {
            assert!(super::super::read_patch(&mut Reader::new(&bytes)).is_err());
        }
        let typed = schema()
            .nodes
            .iter()
            .position(|n| matches!(n.rule, Rule::TypedComponent))
            .unwrap();
        assert!(
            read(
                &mut Reader::new(&[104]),
                typed,
                &mut Budget::new(&Reader::new(&[104])),
                0
            )
            .is_err()
        );
        let dispatcher = schema()
            .nodes
            .iter()
            .position(|n| {
                matches!(
                    n.rule,
                    Rule::Dispatch {
                        branched: false,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(
            read(
                &mut Reader::new(&[127]),
                dispatcher,
                &mut Budget::new(&Reader::new(&[127])),
                0
            )
            .is_err()
        );
        let mut recursive = vec![1, 0, 48];
        for _ in 0..100 {
            recursive.extend([1, 1, 1, 1, 0, 48]);
        }
        recursive.push(0);
        let error = super::super::read_patch(&mut Reader::new(&recursive))
            .unwrap_err()
            .to_string();
        assert!(error.contains("depth limit"), "{error}");
        let mut budget = Budget {
            initial_bytes: 0,
            steps: 0,
        };
        assert!(
            read(&mut Reader::new(&[]), 0, &mut budget, 0)
                .unwrap_err()
                .to_string()
                .contains("work limit")
        );
    }
}
