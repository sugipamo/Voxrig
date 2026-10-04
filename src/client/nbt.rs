//! Native NBT values behind the common item custom-data accessor.
use crate::{Error, ErrorKind, MinecraftVersion, Result};
use anyhow::{Context, bail};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};

/// Java string contents, preserving modified UTF-8's UTF-16 code units.
/// Unpaired surrogates are retained rather than replaced with another character.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NbtString(Vec<u16>);
impl NbtString {
    /// Exact native string code units, including any unpaired surrogate.
    pub fn utf16(&self) -> &[u16] {
        &self.0
    }
    /// Text when all surrogate pairs are valid; otherwise a conversion error.
    pub fn text(&self) -> Result<String> {
        String::from_utf16(&self.0).map_err(|e| Error::new(ErrorKind::InvalidInput, e))
    }
}
impl Serialize for NbtString {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match String::from_utf16(&self.0) {
            Ok(text) => serializer.serialize_str(&text),
            Err(_) => {
                #[derive(Serialize)]
                struct Units<'a> {
                    utf16: &'a [u16],
                }
                Units { utf16: &self.0 }.serialize(serializer)
            }
        }
    }
}
/// One named child of a normalized compound. Wire ordering remains in ItemData.
#[derive(Clone, Debug, Serialize)]
pub struct NbtEntry {
    key: NbtString,
    value: Arc<NbtValue>,
}
impl NbtEntry {
    /// Exact key contents.
    pub fn key(&self) -> &NbtString {
        &self.key
    }
    /// Decoded native child value.
    pub fn value(&self) -> &NbtValue {
        &self.value
    }
}
/// Compound children sorted by native UTF-16 key contents; duplicate keys use the last value.
#[derive(Clone, Debug, Serialize)]
pub struct NbtCompound {
    entries: Vec<NbtEntry>,
}
impl NbtCompound {
    /// Normalized children, without inventing a missing field.
    pub fn entries(&self) -> &[NbtEntry] {
        &self.entries
    }
    /// Lookup a normal Unicode key.
    pub fn get(&self, name: &str) -> Option<&NbtValue> {
        self.get_utf16(&name.encode_utf16().collect::<Vec<_>>())
    }
    /// Lookup an exact native key, including an unpaired surrogate.
    pub fn get_utf16(&self, name: &[u16]) -> Option<&NbtValue> {
        let index = self
            .entries
            .binary_search_by(|entry| entry.key.0.as_slice().cmp(name))
            .ok()?;
        Some(&self.entries[index].value)
    }
}
/// Decoded NBT kinds. Numeric widths, arrays and lists are kept distinct.
/// Float bits preserve non-finite values; decoding follows the owning native version.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum NbtValue {
    /// Signed 8-bit numeric value, also used for native NBT booleans.
    Byte(i8),
    /// Signed 16-bit numeric value.
    Short(i16),
    /// Signed 32-bit numeric value.
    Int(i32),
    /// Signed 64-bit numeric value.
    Long(i64),
    /// Exact decoded float representation; native valueOf canonicalizes signed zero.
    Float {
        /// IEEE-754 bits after native signed-zero canonicalization.
        bits: u32,
    },
    /// Exact decoded double representation; native valueOf canonicalizes signed zero.
    Double {
        /// IEEE-754 bits after native signed-zero canonicalization.
        bits: u64,
    },
    /// Signed bytes, distinct from a list of Byte tags.
    ByteArray(Vec<i8>),
    /// Native UTF-16 string contents.
    String(NbtString),
    /// Native logical list, preserving child order and modern wrapper unboxing.
    List(Vec<Arc<NbtValue>>),
    /// Native named children.
    Compound(NbtCompound),
    /// Signed integers, distinct from a list of Int tags.
    IntArray(Vec<i32>),
    /// Signed longs, distinct from a list of Long tags.
    LongArray(Vec<i64>),
}
impl NbtValue {
    /// Exact Int tag, without numeric-width coercion.
    pub fn as_int(&self) -> Option<i32> {
        if let Self::Int(value) = self {
            Some(*value)
        } else {
            None
        }
    }
    /// Exact native String tag.
    pub fn as_string(&self) -> Option<&NbtString> {
        if let Self::String(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Exact Compound tag.
    pub fn as_compound(&self) -> Option<&NbtCompound> {
        if let Self::Compound(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Exact logical List tag.
    pub fn as_list(&self) -> Option<&[Arc<NbtValue>]> {
        if let Self::List(value) = self {
            Some(value)
        } else {
            None
        }
    }
}
/// Immutable native custom-data compound, interpreted in its selected adapter.
/// This is not an effective item prototype, an action authority or a complete stack hash.
#[derive(Clone, Debug, Serialize)]
pub struct NbtData {
    version: MinecraftVersion,
    root: Arc<NbtValue>,
    persistent_crc32c: Option<u32>,
}
impl NbtData {
    /// Native version controlling NBT comparison and list interpretation.
    pub fn version(&self) -> MinecraftVersion {
        self.version
    }
    /// Complete normalized root compound.
    pub fn root(&self) -> &NbtCompound {
        self.root
            .as_compound()
            .expect("decoder establishes compound root")
    }
    /// Native decoded value equality, distinct from raw ItemData byte equality.
    /// Cross-version comparisons are refused. Independently decoded legacy NaNs
    /// can compare unequal despite identical bytes; shared immutable values retain
    /// native identity equality. This comparison is separate from ItemData's Eq.
    pub fn native_equivalent(&self, other: &Self) -> Result<bool> {
        if self.version != other.version {
            return Err(invalid("NBT values belong to different adapters"));
        }
        Ok(equivalent(&self.root, &other.root, self.version))
    }
    /// Original modern CompoundTag persistent HashOps CRC32C value hash.
    /// Legacy inventory comparison has no such field and returns None.
    /// Does not include item/type/patch/prototype identity or server cache behavior;
    /// matching hashes do not prove value equality or authorize an inventory action.
    pub fn persistent_crc32c(&self) -> Option<u32> {
        self.persistent_crc32c
    }
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, anyhow::anyhow!("{message}"))
}
const MAX_BYTES: usize = 1024 * 1024;
const MAX_NODES: usize = 65_536;
const MAX_DEPTH: usize = 64;
struct Decoder<'a> {
    rest: &'a [u8],
    nodes: usize,
    version: MinecraftVersion,
}
impl<'a> Decoder<'a> {
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        if n > self.rest.len() {
            bail!("truncated NBT value");
        }
        let (field, rest) = self.rest.split_at(n);
        self.rest = rest;
        Ok(field)
    }
    fn byte(&mut self) -> anyhow::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn count(&mut self, width: usize) -> anyhow::Result<usize> {
        let n = usize::try_from(i32::from_be_bytes(self.take(4)?.try_into()?))
            .context("negative NBT count")?;
        if n > MAX_BYTES / width {
            bail!("NBT count exceeds byte budget");
        }
        Ok(n)
    }
    fn string(&mut self) -> anyhow::Result<NbtString> {
        let n = u16::from_be_bytes(self.take(2)?.try_into()?) as usize;
        let bytes = self.take(n)?;
        let mut i = 0;
        let mut units = Vec::with_capacity(n);
        while i < n {
            let a = bytes[i];
            i += 1;
            let unit = if a < 128 {
                u16::from(a)
            } else if a & 224 == 192 {
                let b = *bytes.get(i).context("truncated modified UTF-8")?;
                i += 1;
                if b & 192 != 128 {
                    bail!("invalid modified UTF-8 continuation");
                }
                (u16::from(a & 31) << 6) | u16::from(b & 63)
            } else if a & 240 == 224 {
                let b = *bytes.get(i).context("truncated modified UTF-8")?;
                let c = *bytes.get(i + 1).context("truncated modified UTF-8")?;
                i += 2;
                if b & 192 != 128 || c & 192 != 128 {
                    bail!("invalid modified UTF-8 continuation");
                }
                (u16::from(a & 15) << 12) | (u16::from(b & 63) << 6) | u16::from(c & 63)
            } else {
                bail!("invalid modified UTF-8 lead byte");
            };
            units.push(unit);
        }
        Ok(NbtString(units))
    }
    fn value(&mut self, kind: u8, depth: usize) -> anyhow::Result<Arc<NbtValue>> {
        if depth > MAX_DEPTH || self.nodes == 0 {
            bail!("NBT depth/work budget exceeded");
        }
        self.nodes -= 1;
        let value = match kind {
            1 => NbtValue::Byte(self.byte()? as i8),
            2 => NbtValue::Short(i16::from_be_bytes(self.take(2)?.try_into()?)),
            3 => NbtValue::Int(i32::from_be_bytes(self.take(4)?.try_into()?)),
            4 => NbtValue::Long(i64::from_be_bytes(self.take(8)?.try_into()?)),
            5 => {
                let bits = u32::from_be_bytes(self.take(4)?.try_into()?);
                NbtValue::Float {
                    bits: if f32::from_bits(bits) == 0.0 { 0 } else { bits },
                }
            }
            6 => {
                let bits = u64::from_be_bytes(self.take(8)?.try_into()?);
                NbtValue::Double {
                    bits: if f64::from_bits(bits) == 0.0 { 0 } else { bits },
                }
            }
            7 => {
                let n = self.count(1)?;
                NbtValue::ByteArray(self.take(n)?.iter().map(|b| *b as i8).collect())
            }
            8 => NbtValue::String(self.string()?),
            9 => {
                let kind = self.byte()?;
                let n = self.count(1)?;
                if n > self.nodes || (kind == 0 && n != 0) {
                    bail!("invalid/over-budget NBT list");
                }
                let mut values = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut value = self.value(kind, depth + 1)?;
                    if self.version == MinecraftVersion::Java1_21_11 {
                        if let NbtValue::Compound(compound) = &*value {
                            if compound.entries.len() == 1 && compound.entries[0].key.0.is_empty() {
                                value = compound.entries[0].value.clone();
                            }
                        }
                    }
                    values.push(value);
                }
                NbtValue::List(values)
            }
            10 => {
                let mut entries = BTreeMap::new();
                loop {
                    let kind = self.byte()?;
                    if kind == 0 {
                        break;
                    }
                    let key = self.string()?;
                    let value = self.value(kind, depth + 1)?;
                    entries.insert(key, value);
                }
                NbtValue::Compound(NbtCompound {
                    entries: entries
                        .into_iter()
                        .map(|(key, value)| NbtEntry { key, value })
                        .collect(),
                })
            }
            11 => {
                let n = self.count(4)?;
                let bytes = self.take(n * 4)?;
                NbtValue::IntArray(
                    bytes
                        .chunks_exact(4)
                        .map(|b| i32::from_be_bytes(b.try_into().unwrap()))
                        .collect(),
                )
            }
            12 => {
                let n = self.count(8)?;
                let bytes = self.take(n * 8)?;
                NbtValue::LongArray(
                    bytes
                        .chunks_exact(8)
                        .map(|b| i64::from_be_bytes(b.try_into().unwrap()))
                        .collect(),
                )
            }
            _ => bail!("unknown NBT value type {kind}"),
        };
        Ok(Arc::new(value))
    }
}
pub(crate) fn decode(bytes: &[u8], version: MinecraftVersion) -> Result<NbtData> {
    (|| -> anyhow::Result<NbtData> {
        if bytes.len() > MAX_BYTES {
            bail!("custom NBT exceeds byte budget");
        }
        let mut decoder = Decoder {
            rest: bytes,
            nodes: MAX_NODES,
            version,
        };
        if decoder.byte()? != 10 {
            bail!("custom-data root must be a compound");
        }
        if version == MinecraftVersion::Java1_16_1 {
            decoder.string()?;
        }
        let root = decoder.value(10, 0)?;
        if !decoder.rest.is_empty() {
            bail!("trailing custom NBT bytes");
        }
        let persistent_crc32c = (version == MinecraftVersion::Java1_21_11).then(|| hash(&root));
        Ok(NbtData {
            version,
            root,
            persistent_crc32c,
        })
    })()
    .map_err(|e| Error::new(ErrorKind::InvalidInput, e))
}
/// Logical modern unnamed Tag, including the explicit EndTag sentinel.
/// Component text/codec normalization is separate from this NBT interpretation.
pub(crate) fn decode_unnamed_tag(bytes: &[u8]) -> Result<Option<Arc<NbtValue>>> {
    (|| -> anyhow::Result<_> {
        if bytes.len() > MAX_BYTES {
            bail!("NBT exceeds byte budget");
        }
        let mut decoder = Decoder {
            rest: bytes,
            nodes: MAX_NODES,
            version: MinecraftVersion::Java1_21_11,
        };
        let kind = decoder.byte()?;
        let value = if kind == 0 {
            None
        } else {
            Some(decoder.value(kind, 0)?)
        };
        if !decoder.rest.is_empty() {
            bail!("trailing unnamed NBT bytes");
        }
        Ok(value)
    })()
    .map_err(|e| Error::new(ErrorKind::InvalidInput, e))
}
pub(crate) fn equivalent(
    left: &Arc<NbtValue>,
    right: &Arc<NbtValue>,
    version: MinecraftVersion,
) -> bool {
    if Arc::ptr_eq(left, right) {
        return true;
    }
    match (&**left, &**right) {
        (NbtValue::Byte(a), NbtValue::Byte(b)) => a == b,
        (NbtValue::Short(a), NbtValue::Short(b)) => a == b,
        (NbtValue::Int(a), NbtValue::Int(b)) => a == b,
        (NbtValue::Long(a), NbtValue::Long(b)) => a == b,
        (NbtValue::Float { bits: a }, NbtValue::Float { bits: b }) => {
            a == b && (version == MinecraftVersion::Java1_21_11 || !f32::from_bits(*a).is_nan())
                || version == MinecraftVersion::Java1_21_11
                    && f32::from_bits(*a).is_nan()
                    && f32::from_bits(*b).is_nan()
        }
        (NbtValue::Double { bits: a }, NbtValue::Double { bits: b }) => {
            a == b && (version == MinecraftVersion::Java1_21_11 || !f64::from_bits(*a).is_nan())
                || version == MinecraftVersion::Java1_21_11
                    && f64::from_bits(*a).is_nan()
                    && f64::from_bits(*b).is_nan()
        }
        (NbtValue::String(a), NbtValue::String(b)) => a == b,
        (NbtValue::ByteArray(a), NbtValue::ByteArray(b)) => a == b,
        (NbtValue::IntArray(a), NbtValue::IntArray(b)) => a == b,
        (NbtValue::LongArray(a), NbtValue::LongArray(b)) => a == b,
        (NbtValue::List(a), NbtValue::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b, version))
        }
        (NbtValue::Compound(a), NbtValue::Compound(b)) => {
            a.entries.len() == b.entries.len()
                && a.entries
                    .iter()
                    .zip(&b.entries)
                    .all(|(a, b)| a.key == b.key && equivalent(&a.value, &b.value, version))
        }
        _ => false,
    }
}

// Logical decoded NBT selects native typed inputs for the shared HashOps engine.
fn hash(value: &NbtValue) -> u32 {
    use super::hash_ops::{map, primitive, sequence, string};
    match value {
        NbtValue::Byte(value) => primitive(6, [*value as u8]),
        NbtValue::Short(value) => primitive(7, value.to_le_bytes()),
        NbtValue::Int(value) => primitive(8, value.to_le_bytes()),
        NbtValue::Long(value) => primitive(9, value.to_le_bytes()),
        NbtValue::Float { bits } => primitive(10, bits.to_le_bytes()),
        NbtValue::Double { bits } => primitive(11, bits.to_le_bytes()),
        NbtValue::String(value) => string(&value.0),
        NbtValue::ByteArray(values) => sequence(14, 15, values.iter().map(|v| *v as u8)),
        NbtValue::IntArray(values) => sequence(16, 17, values.iter().flat_map(|v| v.to_le_bytes())),
        NbtValue::LongArray(values) => {
            sequence(18, 19, values.iter().flat_map(|v| v.to_le_bytes()))
        }
        NbtValue::List(values) => sequence(4, 5, values.iter().flat_map(|v| hash(v).to_le_bytes())),
        NbtValue::Compound(values) => map(values
            .entries
            .iter()
            .map(|entry| (string(&entry.key.0), hash(&entry.value)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn corpus(version: MinecraftVersion) -> Value {
        serde_json::from_str(match version {
            MinecraftVersion::Java1_16_1 => {
                include_str!("../../data/client_api/nbt_semantics-1.16.1.json")
            }
            MinecraftVersion::Java1_21_11 => {
                include_str!("../../data/client_api/nbt_semantics-1.21.11.json")
            }
        })
        .unwrap()
    }
    #[test]
    fn every_original_unnamed_tag_kind_uses_the_same_logical_nbt_decoder() {
        use std::io::Read;
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../data/client_api/component_value_cases-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: Value = serde_json::from_slice(&bytes).unwrap();
        let tags = facts["unnamed_tags"].as_array().unwrap();
        for row in tags {
            let bytes = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
            let value = decode_unnamed_tag(&bytes).unwrap();
            let (decoded, crc) = match value {
                Some(value) => (describe(&value), hash(&value)),
                None => (json!({"kind":0}), crate::client::hash_ops::primitive(1, [])),
            };
            assert_eq!(decoded, row["decoded"], "{}", row["input_hex"]);
            assert_eq!(crc as i32, row["pure_nbt_crc32c"].as_i64().unwrap() as i32);
            let mut trailing = bytes.clone();
            trailing.push(0);
            assert!(decode_unnamed_tag(&trailing).is_err());
            for end in 0..bytes.len() {
                assert!(decode_unnamed_tag(&bytes[..end]).is_err());
            }
        }
        assert_eq!(tags.len(), 20);
    }
    // Only translates public decoded fields into the original oracle's JSON shape.
    fn describe(value: &NbtValue) -> Value {
        match value {
            NbtValue::Byte(v) => json!({"kind":1,"value":v}),
            NbtValue::Short(v) => json!({"kind":2,"value":v}),
            NbtValue::Int(v) => json!({"kind":3,"value":v}),
            NbtValue::Long(v) => json!({"kind":4,"value":v}),
            NbtValue::Float { bits } => json!({"kind":5,"bits":bits}),
            NbtValue::Double { bits } => json!({"kind":6,"bits":bits.to_string()}),
            NbtValue::ByteArray(v) => json!({"kind":7,"values":v}),
            NbtValue::String(v) => json!({"kind":8,"units":v.utf16()}),
            NbtValue::List(v) => {
                json!({"kind":9,"values":v.iter().map(|v|describe(v)).collect::<Vec<_>>()})
            }
            NbtValue::Compound(v) => {
                json!({"kind":10,"entries":v.entries().iter().map(|e|json!({"key_units":e.key().utf16(),"value":describe(e.value())})).collect::<Vec<_>>()})
            }
            NbtValue::IntArray(v) => json!({"kind":11,"values":v}),
            NbtValue::LongArray(v) => json!({"kind":12,"values":v}),
        }
    }
    #[test]
    fn original_nbt_decoders_equality_and_persistent_hashes_match_all_native_cases() {
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let data = corpus(version);
            let mut decoded = BTreeMap::new();
            assert_eq!(data["values"].as_array().unwrap().len(), 93);
            assert_eq!(data["pairs"].as_array().unwrap().len(), 4371);
            for row in data["values"].as_array().unwrap() {
                let name = row["name"].as_str().unwrap();
                let bytes = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
                let value = decode(&bytes, version).unwrap();
                assert_eq!(describe(&value.root), row["decoded"], "{version:?} {name}");
                let canonical = decode(
                    &hex::decode(row["canonical_hex"].as_str().unwrap()).unwrap(),
                    version,
                )
                .unwrap();
                assert_eq!(
                    value.native_equivalent(&canonical).unwrap(),
                    row["canonical_decode_equal"].as_bool().unwrap(),
                    "canonical {version:?} {name}"
                );
                if version == MinecraftVersion::Java1_21_11 {
                    assert_eq!(
                        value.persistent_crc32c().unwrap() as i32,
                        row["persistent_crc32c"].as_i64().unwrap() as i32,
                        "hash {name}"
                    );
                } else {
                    assert!(value.persistent_crc32c().is_none());
                }
                for prefix in 0..bytes.len() {
                    assert!(
                        decode(&bytes[..prefix], version).is_err(),
                        "{name} prefix {prefix}"
                    );
                }
                let mut trailing = bytes;
                trailing.push(0);
                assert!(decode(&trailing, version).is_err());
                decoded.insert(name.to_owned(), value);
            }
            for pair in data["pairs"].as_array().unwrap() {
                let a = pair["left"].as_str().unwrap();
                let b = pair["right"].as_str().unwrap();
                assert_eq!(
                    decoded[a].native_equivalent(&decoded[b]).unwrap(),
                    pair["equal"].as_bool().unwrap(),
                    "{version:?} {a} vs {b}"
                );
            }
            for row in data["failures"].as_array().unwrap() {
                assert!(
                    decode(
                        &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
                        version
                    )
                    .is_err(),
                    "native rejection {}",
                    row["name"]
                );
            }
        }
    }
    #[test]
    fn custom_data_versions_budget_missing_values_and_surrogate_contents_remain_distinct() {
        let a = decode(&[10, 0, 0, 0], MinecraftVersion::Java1_16_1).unwrap();
        let b = decode(&[10, 0], MinecraftVersion::Java1_21_11).unwrap();
        assert!(a.native_equivalent(&b).is_err());
        assert!(a.root().get("missing").is_none());
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            assert!(decode(&[0], version).is_err());
            let mut deep = if version == MinecraftVersion::Java1_16_1 {
                vec![10, 0, 0]
            } else {
                vec![10]
            };
            for _ in 0..66 {
                deep.extend([10, 0, 0]);
            }
            deep.extend([0; 67]);
            assert!(decode(&deep, version).is_err());
            let mut excessive = if version == MinecraftVersion::Java1_16_1 {
                vec![10, 0, 0]
            } else {
                vec![10]
            };
            for _ in 0..MAX_NODES {
                excessive.extend([1, 0, 0, 1]);
            }
            excessive.push(0);
            assert!(decode(&excessive, version).is_err());
            assert!(decode(&vec![0; MAX_BYTES + 1], version).is_err());
        }
        let s = NbtString(vec![0xd800]);
        assert!(s.text().is_err());
        assert_eq!(serde_json::to_value(s).unwrap(), json!({"utf16":[55296]}));
        let s = NbtString("a\0😀".encode_utf16().collect());
        assert_eq!(s.text().unwrap(), "a\0😀");
    }
    #[test]
    fn original_nbt_facts_and_default_custom_data_prototypes_are_source_bound() {
        use sha2::{Digest, Sha256};
        let source: Value = serde_json::from_str(include_str!(
            "../../data/client_api/nbt_semantics_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for (path, expected) in source["generators_sha256"].as_object().unwrap() {
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(root.join(path)).unwrap())
                ),
                expected.as_str().unwrap(),
                "{path}"
            );
        }
        for run in source["runs"].as_array().unwrap() {
            for (path, expected) in run["files_sha256"].as_object().unwrap() {
                assert_eq!(
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(root.join(path)).unwrap())
                    ),
                    expected.as_str().unwrap(),
                    "{path}"
                );
            }
        }
        let modern = corpus(MinecraftVersion::Java1_21_11);
        assert_eq!(modern["default_custom_data_prototypes"]["items"], 1505);
        assert!(
            modern["default_custom_data_prototypes"]["items_with_custom_data"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn one_common_item_custom_data_consumer_preserves_wire_and_rejects_wrong_identity() {
        use crate::client::registry::Registry;
        use crate::client::{ItemComponent, ItemComponentPatch, ItemData, ItemStack};
        for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
            let registry = Registry::for_version(version);
            let definition = registry.item("minecraft:stone").unwrap();
            let case = corpus(version);
            let row = case["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == "integer-3-1")
                .unwrap();
            let bytes = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
            let data = match version {
                MinecraftVersion::Java1_16_1 => ItemData::LegacyNbt { bytes },
                MinecraftVersion::Java1_21_11 => ItemData::ModernComponents {
                    patch: ItemComponentPatch {
                        added: vec![ItemComponent {
                            definition: registry.item_component("minecraft:custom_data").unwrap(),
                            bytes,
                        }],
                        removed: vec![],
                    },
                },
            };
            let mut item = ItemStack {
                id: definition.id,
                name: definition.name,
                count: 1,
                data,
            };
            let before = item.data.clone();
            let custom = item.custom_data().unwrap().unwrap();
            assert_eq!(custom.version(), version);
            assert_eq!(custom.root().get("value").unwrap().as_int(), Some(1));
            assert_eq!(item.data, before);
            let second = item.custom_data().unwrap().unwrap();
            assert!(custom.native_equivalent(&second).unwrap());
            item.name = "minecraft:dirt".into();
            assert!(item.custom_data().is_err());
            item.name = "minecraft:stone".into();
            if let ItemData::ModernComponents { patch } = &mut item.data {
                patch.added.push(patch.added[0].clone());
                assert!(item.custom_data().is_err());
                let ItemData::ModernComponents { patch } = &mut item.data else {
                    unreachable!()
                };
                patch.added.pop();
                patch.added[0].definition.name = "minecraft:damage".into();
                assert!(item.custom_data().is_err());
            }
            item.data = ItemData::Default;
            assert!(item.custom_data().unwrap().is_none());
            item.data = if version == MinecraftVersion::Java1_16_1 {
                ItemData::ModernComponents {
                    patch: ItemComponentPatch {
                        added: vec![],
                        removed: vec![],
                    },
                }
            } else {
                ItemData::LegacyNbt {
                    bytes: vec![10, 0, 0, 0],
                }
            };
            assert!(item.custom_data().is_err());
            if version == MinecraftVersion::Java1_21_11 {
                item.data = ItemData::ModernComponents {
                    patch: ItemComponentPatch {
                        added: vec![],
                        removed: vec![registry.item_component("minecraft:custom_data").unwrap()],
                    },
                };
                assert!(item.custom_data().unwrap().is_none());
            }
        }
    }
}
