//! Typed persistent HashOps primitives, shared by decoded NBT and component work.
//! Inputs here are codec values, never raw item wire bytes or action authority.
struct Crc32c(u32);
impl Crc32c {
    fn new() -> Self {
        Self(u32::MAX)
    }
    fn byte(&mut self, byte: u8) {
        self.0 ^= u32::from(byte);
        for _ in 0..8 {
            self.0 = (self.0 >> 1) ^ (0x82f63b78 & 0u32.wrapping_sub(self.0 & 1));
        }
    }
    fn bytes(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            self.byte(byte);
        }
    }
    fn finish(self) -> u32 {
        !self.0
    }
}

// Callers supply the original codec's typed marker and native-width LE values.
pub(super) fn primitive(marker: u8, bytes: impl IntoIterator<Item = u8>) -> u32 {
    let mut crc = Crc32c::new();
    crc.byte(marker);
    crc.bytes(bytes);
    crc.finish()
}
pub(super) fn sequence(start: u8, end: u8, bytes: impl IntoIterator<Item = u8>) -> u32 {
    let mut crc = Crc32c::new();
    crc.byte(start);
    crc.bytes(bytes);
    crc.byte(end);
    crc.finish()
}
pub(super) fn string(units: &[u16]) -> u32 {
    let mut crc = Crc32c::new();
    crc.byte(12);
    crc.bytes((units.len() as u32).to_le_bytes());
    for unit in units {
        crc.bytes(unit.to_le_bytes());
    }
    crc.finish()
}
pub(super) fn map(entries: impl IntoIterator<Item = (u32, u32)>) -> u32 {
    let mut entries = entries.into_iter().collect::<Vec<_>>();
    // Original HashCode ordering is unsigned LE u32, then the value hash.
    // Duplicate entries stay duplicate; hash equality does not imply value equality.
    entries.sort_unstable();
    sequence(
        2,
        3,
        entries
            .into_iter()
            .flat_map(|(k, v)| k.to_le_bytes().into_iter().chain(v.to_le_bytes())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use serde::Deserialize;
    use serde_json::Value;
    use std::io::Read;

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum Node {
        Empty,
        Byte { value: i8 },
        Short { value: i16 },
        Int { value: i32 },
        Long { value: i64 },
        Float { bits: u32 },
        Double { bits: String },
        Boolean { value: bool },
        String { units: Vec<u16> },
        List { values: Vec<usize> },
        Map { entries: Vec<[usize; 2]> },
        ByteArray { values: Vec<u8> },
        IntArray { values: Vec<i32> },
        LongArray { values: Vec<i64> },
    }
    fn corpus() -> Value {
        let mut bytes = Vec::new();
        GzDecoder::new(&include_bytes!("../../data/client_api/item_semantics-1.21.11.json.gz")[..])
            .read_to_end(&mut bytes)
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    #[test]
    fn native_semantic_facts_are_bound_to_original_inputs_and_own_tools() {
        use sha2::{Digest, Sha256};
        let source: Value = serde_json::from_str(include_str!(
            "../../data/client_api/item_semantics_source.json"
        ))
        .unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let check = |path: &str, hash: &Value| {
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(root.join(path)).unwrap())
                ),
                hash.as_str().unwrap(),
                "{path}"
            );
        };
        for (path, hash) in source["generators_sha256"].as_object().unwrap() {
            check(path, hash);
        }
        for run in source["runs"].as_array().unwrap() {
            for (path, hash) in run["files_sha256"].as_object().unwrap() {
                check(path, hash);
            }
        }
        let mut bytes = Vec::new();
        GzDecoder::new(&include_bytes!("../../data/client_api/item_semantics-1.16.1.json.gz")[..])
            .read_to_end(&mut bytes)
            .unwrap();
        let legacy: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(legacy["items"].as_array().unwrap().len(), 4805);
        assert_eq!(legacy["item_equivalence_groups"], 3595);
        assert_eq!(
            legacy["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["independent_decode_matches"] == false)
                .count(),
            126
        );
        assert_eq!(legacy["prototype_cases"].as_array().unwrap().len(), 1428);
    }
    #[test]
    fn typed_hashes_match_all_original_component_encoder_inputs() {
        let data = corpus();
        let nodes: Vec<Node> = serde_json::from_value(data["hash_nodes"].clone()).unwrap();
        let mut hashes: Vec<u32> = Vec::new();
        for node in nodes {
            let child = |index: usize| {
                *hashes
                    .get(index)
                    .expect("original DAG must reference earlier nodes")
            };
            let hash = match node {
                Node::Empty => primitive(1, []),
                Node::Byte { value } => primitive(6, [value as u8]),
                Node::Short { value } => primitive(7, value.to_le_bytes()),
                Node::Int { value } => primitive(8, value.to_le_bytes()),
                Node::Long { value } => primitive(9, value.to_le_bytes()),
                Node::Float { bits } => primitive(10, bits.to_le_bytes()),
                Node::Double { bits } => primitive(11, bits.parse::<u64>().unwrap().to_le_bytes()),
                Node::Boolean { value } => primitive(13, [u8::from(value)]),
                Node::String { units } => string(&units),
                Node::List { values } => sequence(
                    4,
                    5,
                    values.into_iter().flat_map(|i| child(i).to_le_bytes()),
                ),
                Node::Map { entries } => {
                    map(entries.into_iter().map(|[k, v]| (child(k), child(v))))
                }
                Node::ByteArray { values } => sequence(14, 15, values),
                Node::IntArray { values } => {
                    sequence(16, 17, values.into_iter().flat_map(i32::to_le_bytes))
                }
                Node::LongArray { values } => {
                    sequence(18, 19, values.into_iter().flat_map(i64::to_le_bytes))
                }
            };
            hashes.push(hash);
        }
        let rows = data["components"].as_array().unwrap();
        let mut successes = 0;
        for row in rows {
            for (node, expected) in [
                ("hash_node", "persistent_crc32c"),
                ("fresh_hash_node", "fresh_persistent_crc32c"),
                ("outer_cached_hash_node", "outer_cached_crc32c"),
            ] {
                if let Some(index) = row[node].as_u64() {
                    assert_eq!(
                        hashes[index as usize] as i32,
                        row[expected].as_i64().unwrap() as i32,
                        "{expected}: {}",
                        row["case"]
                    );
                    successes += 1;
                }
            }
        }
        assert_eq!(rows.len(), 4134);
        assert_eq!(hashes.len(), 5846);
        assert_eq!(successes, 8335);
        assert_eq!(
            rows.iter()
                .map(|r| r["component"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            104
        );
    }
    #[test]
    fn native_value_equality_and_cached_nan_hash_are_distinct_facts() {
        let data = corpus();
        let rows = data["components"].as_array().unwrap();
        let get = |case: &str| {
            rows.iter()
                .find(|r| r["case"].as_str() == Some(case))
                .unwrap()
        };
        let canonical = get("nbt-float-5-0x7fc00000");
        let alternate = get("nbt-float-5-0x7fc00001");
        assert_eq!(canonical["group"], alternate["group"]);
        assert_ne!(
            canonical["persistent_crc32c"],
            alternate["persistent_crc32c"]
        );
        assert_eq!(
            canonical["outer_cached_crc32c"],
            alternate["outer_cached_crc32c"]
        );
        assert_eq!(canonical["hashed_stack_hex"], alternate["hashed_stack_hex"]);
        assert_eq!(
            rows.iter()
                .filter(|r| r.get("native_encoder_error").is_some())
                .count(),
            13
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.get("native_hashed_stack_error").is_some())
                .count(),
            13
        );
        assert_eq!(
            data["prototype_cases"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["same_item_data"] == true && r["matches"] == true)
                .count(),
            24789
        );
    }
}
