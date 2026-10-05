//! Native component registry and bounded, lossless received patch framing.
//! Payload framing is distinct from gameplay interpretation and stack hashing.
use super::wire::Reader;
use crate::client::{ItemComponent, ItemComponentPatch, registry::Registry};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, sync::OnceLock};
mod framing;
mod profile;
#[cfg(test)]
mod profile_tests;
#[cfg(test)]
mod selector_tests;
mod text;
#[cfg(test)]
mod text_tests;
#[cfg(test)]
mod uri_tests;
mod values;

#[cfg(test)]
mod native_evidence_tests;

#[derive(serde::Deserialize)]
pub(crate) struct Definition {
    pub(crate) name: String,
    pub(crate) native_id: i32,
    stream_codec_class: String,
}
pub(crate) fn definitions() -> &'static [Definition] {
    static DEFINITIONS: OnceLock<Vec<Definition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../data/client_api/item_components-1.21.11.json"
        ))
        .expect("valid pinned original component registry")
    })
}
fn definition(id: i32) -> Result<&'static Definition> {
    definitions()
        .iter()
        .find(|d| d.native_id == id)
        .context("unknown native item-component ID; update Voxrig")
}
/// Validate a public patch's identities and complete encoded field boundaries.
/// This does not resolve registry references, normalize values or authorize actions.
pub(crate) fn validate_patch(patch: &ItemComponentPatch) -> Result<()> {
    if patch.added.len() + patch.removed.len() > definitions().len() {
        bail!("item-component patch exceeds native type count");
    }
    let size = patch.added.iter().try_fold(0usize, |sum, field| {
        sum.checked_add(field.bytes.len())
            .context("item-component patch byte overflow")
    })?;
    if size > 1_048_576 {
        bail!("item-component patch exceeds byte budget");
    }
    let mut bytes = Vec::with_capacity(size + 104 * 5 + 10);
    crate::protocol::put_varint(&mut bytes, patch.added.len() as i32);
    crate::protocol::put_varint(&mut bytes, patch.removed.len() as i32);
    for field in &patch.added {
        crate::protocol::put_varint(&mut bytes, field.definition.id.value());
        bytes.extend_from_slice(&field.bytes);
    }
    for field in &patch.removed {
        crate::protocol::put_varint(&mut bytes, field.id.value());
    }
    let decoded = decode_patch(&bytes)?;
    if decoded != *patch {
        bail!("item-component identities or field boundaries disagree");
    }
    Ok(())
}
/// Decode a complete native signed scalar through the same typed grammar.
pub(crate) fn scalar_value(value: &ItemComponent) -> Result<i32> {
    let native = definition(value.definition.id.value())?;
    if value.definition.name != native.name {
        bail!("item-component name/ID mismatch");
    }
    match framing::decode_value(native, &value.bytes)? {
        values::Value::Integer(value) => Ok(value),
        _ => bail!("item-component value is not a signed integer"),
    }
}
/// Decode one complete patch field, without exposing adapter wire internals.
pub(crate) fn decode_patch(bytes: &[u8]) -> Result<ItemComponentPatch> {
    let mut reader = Reader::new(bytes);
    let patch = read_patch(&mut reader)?.context("unknown item-component value")?;
    reader.end()?;
    Ok(patch)
}
/// Decode original encoded field boundaries, retaining values without semantic normalization.
pub(crate) fn read_patch(r: &mut Reader<'_>) -> Result<Option<ItemComponentPatch>> {
    let added = r.count(definitions().len())?;
    let removed = r.count(definitions().len())?;
    if added + removed > definitions().len() {
        bail!("item-component patch exceeds native type count");
    }
    let registry = Registry::for_version(crate::MinecraftVersion::Java1_21_11);
    let mut seen = BTreeSet::new();
    let mut patch = ItemComponentPatch {
        added: Vec::with_capacity(added),
        removed: Vec::with_capacity(removed),
    };
    let mut budget = framing::Budget::new(r);
    for _ in 0..added {
        let id = r.varint()?;
        let native = definition(id)?;
        if !seen.insert(id) {
            bail!("duplicate item-component patch type");
        }
        let before = r.remaining();
        framing::value(r, native, &mut budget, 0)?;
        let length = before.len() - r.remaining().len();

        patch.added.push(ItemComponent {
            definition: registry.item_component_by_native_id(id)?,
            bytes: before[..length].to_vec(),
        });
    }
    for _ in 0..removed {
        let id = r.varint()?;
        definition(id)?;
        if !seen.insert(id) {
            bail!("duplicate item-component patch type");
        }
        patch
            .removed
            .push(registry.item_component_by_native_id(id)?);
    }
    Ok(Some(patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn cases() -> Value {
        serde_json::from_str(include_str!(
            "../../../data/client_api/item_component_cases-1.21.11.json"
        ))
        .unwrap()
    }
    #[test]
    fn original_component_values_and_all_removal_patches_are_framed_exactly() {
        let corpus = cases();
        assert_eq!(definitions().len(), 104);
        assert!(corpus["failures"].as_array().unwrap().is_empty());
        let mut covered = BTreeSet::new();
        for sample in corpus["samples"].as_array().unwrap() {
            let bytes = hex::decode(sample["patch_hex"].as_str().unwrap()).unwrap();
            let mut reader = Reader::new(&bytes);
            let patch = read_patch(&mut reader).unwrap();
            reader.end().unwrap();
            let patch = patch.unwrap();
            assert_eq!(patch.added.len(), 1);
            assert!(patch.removed.is_empty());
            let added = &patch.added[0];
            assert_eq!(added.definition.name, sample["name"].as_str().unwrap());
            assert_eq!(
                added.definition.id.value(),
                sample["native_id"].as_i64().unwrap() as i32
            );
            assert_eq!(
                hex::encode(&added.bytes),
                sample["value_hex"].as_str().unwrap()
            );
            covered.insert(added.definition.name.clone());
            for end in 0..bytes.len() {
                assert!(
                    read_patch(&mut Reader::new(&bytes[..end])).is_err(),
                    "{} prefix {end}",
                    sample["name"]
                );
            }
        }
        for removed in corpus["removed"].as_array().unwrap() {
            let bytes = hex::decode(removed["patch_hex"].as_str().unwrap()).unwrap();
            let mut reader = Reader::new(&bytes);
            let patch = read_patch(&mut reader).unwrap().unwrap();
            reader.end().unwrap();
            assert!(patch.added.is_empty());
            assert_eq!(patch.removed.len(), 1);
            assert_eq!(patch.removed[0].name, removed["name"].as_str().unwrap());
            assert_eq!(
                patch.removed[0].id.value(),
                removed["native_id"].as_i64().unwrap() as i32
            );
            for end in 0..bytes.len() {
                assert!(read_patch(&mut Reader::new(&bytes[..end])).is_err());
            }
        }
        assert_eq!(covered.len(), 104);
    }
    #[test]
    fn original_mixed_patch_preserves_values_and_equality_ignores_type_order() {
        let corpus = cases();
        let original = &corpus["mixed"];
        let bytes = hex::decode(original["patch_hex"].as_str().unwrap()).unwrap();
        let mut reader = Reader::new(&bytes);
        let patch = read_patch(&mut reader).unwrap().unwrap();
        reader.end().unwrap();
        assert_eq!(patch.added.len(), 6);
        assert_eq!(patch.removed.len(), 1);
        assert_eq!(patch.removed[0].name, original["removed"].as_str().unwrap());
        for value in original["added"].as_array().unwrap() {
            let added = patch
                .added
                .iter()
                .find(|c| c.definition.name == value["name"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                added.definition.id.value(),
                value["native_id"].as_i64().unwrap() as i32
            );
            assert_eq!(
                hex::encode(&added.bytes),
                value["value_hex"].as_str().unwrap()
            );
        }
        let mut reordered = patch.clone();
        reordered.added.reverse();
        assert_eq!(reordered, patch);
        assert_ne!(
            serde_json::to_value(&reordered).unwrap(),
            serde_json::to_value(&patch).unwrap()
        );
        reordered.added[0].bytes.push(0);
        assert_ne!(reordered, patch);
        let mut lost_removal = patch.clone();
        lost_removal.removed.clear();
        assert_ne!(lost_removal, patch);
    }
    #[test]
    fn duplicates_unknown_ids_invalid_counts_and_unbounded_payloads_are_refused() {
        // Two known zero-byte unit additions still cannot duplicate a type;
        // adding and removing the same type is also refused before projection.
        for invalid in [
            vec![2, 0, 4, 4],
            vec![1, 1, 4, 4],
            vec![0, 2, 4, 4],
            vec![0, 1, 104],
            vec![0, 1, 255, 255, 255, 255, 7],
            vec![105, 0],
            vec![104, 1],
        ] {
            assert!(
                read_patch(&mut Reader::new(&invalid)).is_err(),
                "{invalid:?}"
            );
        }
        // The unchanged native boolean codec accepts any nonzero byte.
        let noncanonical = read_patch(&mut Reader::new(&[1, 0, 21, 2]))
            .unwrap()
            .unwrap();
        assert_eq!(noncanonical.added[0].bytes, [2]);
        assert!(read_patch(&mut Reader::new(&[1, 0, 0, 9, 0, 0, 0, 0, 1])).is_err());
        let mut oversized = vec![1, 0, 0, 7];
        oversized.extend(1_048_577_i32.to_be_bytes());
        oversized.resize(oversized.len() + 1_048_577, 0);
        assert!(read_patch(&mut Reader::new(&oversized)).is_err());
    }
    #[test]
    fn pinned_registry_and_native_evidence_are_bound_to_their_own_sources() {
        use sha2::{Digest, Sha256};
        let source: Value = serde_json::from_str(include_str!(
            "../../../data/client_api/item_component_source.json"
        ))
        .unwrap();
        for (path, bytes) in [
            (
                "data/client_api/item_components-1.21.11.json",
                include_bytes!("../../../data/client_api/item_components-1.21.11.json").as_slice(),
            ),
            (
                "data/client_api/item_component_cases-1.21.11.json",
                include_bytes!("../../../data/client_api/item_component_cases-1.21.11.json")
                    .as_slice(),
            ),
        ] {
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                source["files_sha256"][path].as_str().unwrap()
            );
        }
        for (path, bytes) in [
            (
                "scripts/ExportItemComponents.java",
                include_bytes!("../../../scripts/ExportItemComponents.java").as_slice(),
            ),
            (
                "scripts/ExportInventoryTransfers.java",
                include_bytes!("../../../scripts/ExportInventoryTransfers.java").as_slice(),
            ),
            (
                "scripts/export_item_components.py",
                include_bytes!("../../../scripts/export_item_components.py").as_slice(),
            ),
            (
                "scripts/export_regular_clicks.py",
                include_bytes!("../../../scripts/export_regular_clicks.py").as_slice(),
            ),
            (
                "data/client_api/item_component_requests-1.21.11.json",
                include_bytes!("../../../data/client_api/item_component_requests-1.21.11.json")
                    .as_slice(),
            ),
        ] {
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                source["generators_sha256"][path].as_str().unwrap()
            );
        }
        assert_eq!(source["component_types"], 104);
        assert_eq!(source["removal_roundtrips"], 104);
        assert_eq!(source["component_roundtrips"], 518);
        assert_eq!(source["packet_roundtrips"], 2_590);
        assert_eq!(source["registry_binding_fixture_count"], 133);
    }
}
