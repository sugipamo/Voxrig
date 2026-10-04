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
}
pub(super) fn value(
    r: &mut Reader<'_>,
    native: &Definition,
    budget: &mut Budget,
    depth: usize,
) -> Result<()> {
    let root = *schema()
        .roots
        .get(&native.name)
        .context("missing pinned component codec")?;
    if schema().nodes[root].native_class != native.stream_codec_class {
        bail!("pinned component codec composition differs from registry");
    }
    read(r, root, budget, depth)
}
fn reference(r: &mut Reader<'_>, registry: &str) -> Result<i32> {
    let id = r.varint()?;
    if id < 0 || !registry.starts_with("minecraft:") {
        bail!("invalid encoded registry reference: {registry}/{id}");
    }
    // Preserve the original ID. This alone does not resolve a datapack-dependent
    // registry, confer semantic identity, or authorize any item operation.
    Ok(id)
}
fn text(r: &mut Reader<'_>, maximum: usize) -> Result<()> {
    let value = r.string()?;
    if value.encode_utf16().count() > maximum {
        bail!("native item-component string length limit");
    }
    Ok(())
}
fn nested_patch(r: &mut Reader<'_>, budget: &mut Budget, depth: usize) -> Result<()> {
    let added = r.count(definitions().len())?;
    let removed = r.count(definitions().len())?;
    if added + removed > definitions().len() {
        bail!("nested item-component patch type count limit");
    }
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..added {
        let id = r.varint()?;
        let native = definition(id)?;
        if !seen.insert(id) {
            bail!("duplicate nested item-component type");
        }
        value(r, native, budget, depth + 1)?;
    }
    for _ in 0..removed {
        let id = r.varint()?;
        definition(id)?;
        if !seen.insert(id) {
            bail!("duplicate nested item-component type");
        }
    }
    budget.check(r)
}

fn read(r: &mut Reader<'_>, node: usize, budget: &mut Budget, depth: usize) -> Result<()> {
    if depth > 256 {
        bail!("item-component codec depth limit");
    }
    budget.steps = budget
        .steps
        .checked_sub(1)
        .context("item-component work limit")?;
    budget.check(r)?;
    let child_depth = depth + 1;
    match &schema()
        .nodes
        .get(node)
        .context("invalid pinned codec node")?
        .rule
    {
        Rule::Sequence { children } => {
            for &child in children {
                read(r, child, budget, child_depth)?;
            }
        }
        Rule::Forward { child } => read(r, *child, budget, child_depth)?,
        Rule::Boolean => {
            r.bool()?;
        }
        Rule::Fixed { length } => {
            r.take(*length)?;
        }
        Rule::Varint => {
            r.varint()?;
        }
        Rule::Nbt => r.skip_nbt()?,
        Rule::Unit => {}
        Rule::String { maximum } => text(r, *maximum)?,
        Rule::Optional { child } => {
            if r.bool()? {
                read(r, *child, budget, child_depth)?;
            }
        }
        Rule::List { child, maximum } => {
            for _ in 0..r.count((*maximum).min(65_536))? {
                read(r, *child, budget, child_depth)?;
            }
        }
        Rule::Map {
            key,
            value,
            maximum,
        } => {
            for _ in 0..r.count((*maximum).min(65_536))? {
                read(r, *key, budget, child_depth)?;
                read(r, *value, budget, child_depth)?;
            }
        }
        Rule::Either { left, right } => {
            let child = if r.bool()? { left } else { right };
            read(r, *child, budget, child_depth)?;
        }
        Rule::Registry { registry } => {
            reference(r, registry)?;
        }
        Rule::Holder { registry, inline } => {
            if reference(r, registry)? == 0 {
                read(r, *inline, budget, child_depth)?;
            }
        }
        Rule::HolderSet { registry, child } => {
            let count = reference(r, registry)?;
            if count == 0 {
                text(r, 32_767)?;
            } else {
                let count = usize::try_from(count - 1)?;
                if count > 65_536 {
                    bail!("item-component holder-set count limit");
                }
                for _ in 0..count {
                    read(r, *child, budget, child_depth)?;
                }
            }
        }
        Rule::ProfileProperties => {
            for _ in 0..r.count(65_536)? {
                text(r, 64)?;
                text(r, 32_767)?;
                if r.bool()? {
                    text(r, 32_767)?;
                }
            }
        }
        Rule::TypedComponent => {
            let native = definition(r.varint()?)?;
            value(r, native, budget, child_depth)?;
        }
        Rule::Item { nonempty } => {
            let count = r.varint()?;
            if count < 0 || (*nonempty && count == 0) {
                bail!("invalid nested native item count");
            }
            if count != 0 {
                let id = r.varint()?;
                if *nonempty && id == 0 {
                    bail!("empty air stack in nonempty nested item codec");
                }
                crate::client::registry::Registry::for_version(
                    crate::MinecraftVersion::Java1_21_11,
                )
                .item_by_native_id(id)?;
                nested_patch(r, budget, child_depth)?;
            }
        }
        Rule::Patch => nested_patch(r, budget, child_depth)?,
        Rule::Dispatch { branched, variants } => {
            let left = if *branched { Some(r.bool()?) } else { None };
            let tag = r.varint()?;
            let chosen = variants
                .iter()
                .find(|v| v.tag == tag && v.left == left)
                .context("unknown native component dispatcher tag; update Voxrig")?;
            read(r, chosen.codec, budget, child_depth)?;
        }
    }
    budget.check(r)
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
