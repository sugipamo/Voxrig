use super::{framing, values::Value};
use crate::client::{
    nbt,
    profile::{Kind, Profile, Texture},
};
use serde_json::{Value as Json, json};
use std::io::Read;

pub(super) fn fields(profile: &Profile) -> Json {
    let mut value = match &profile.kind {
        Kind::DynamicName { name } => {
            json!({"class":"doy$a", "left":true,"name":name,"properties":[]})
        }
        Kind::DynamicId { id } => json!({"class":"doy$a", "left":false,"id":id,"properties":[]}),
        Kind::Full {
            name,
            id,
            properties,
        } => json!({"class":"doy$c","left":true,"name":name,"id":id,"properties":properties}),
        Kind::Partial {
            name,
            id,
            properties,
        } => json!({"class":"doy$c","left":false,"name":name,"id":id,"properties":properties}),
    };
    fn texture(value: &Option<Texture>) -> Json {
        value.as_ref().map(|v| json!({"id":format!("{}:{}",v.id.namespace,v.id.path),"path":format!("{}:{}",v.path.namespace,v.path.path)})).unwrap_or(Json::Null)
    }
    value["skin"] = json!({"texture":texture(&profile.skin.texture),"cape":texture(&profile.skin.cape),"elytra":texture(&profile.skin.elytra),"model":profile.skin.model});
    value
}
fn units(value: &Json) -> Vec<u16> {
    value
        .as_str()
        .map(|v| v.encode_utf16().collect())
        .unwrap_or_else(|| {
            value["utf16"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u16)
                .collect()
        })
}
fn stream(bytes: &[u8]) -> anyhow::Result<Box<Profile>> {
    match framing::decode_node(583, bytes)? {
        Value::Profile(value) => Ok(value),
        _ => anyhow::bail!("profile root not normalized"),
    }
}
#[test]
fn original_profile_constructors_fields_and_all_equal_pairs_match() {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../../../data/client_api/profile_cases-1.21.11.json.gz")[..],
    )
    .read_to_end(&mut bytes)
    .unwrap();
    let facts: Json = serde_json::from_slice(&bytes).unwrap();
    let mut values = Vec::new();
    let mut accepted = 0;
    for row in facts["cases"].as_array().unwrap() {
        let bytes = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
        let result = if row["format"] == "nbt" {
            nbt::decode_unnamed_tag(&bytes)
                .map_err(anyhow::Error::from)
                .and_then(|v| crate::client::profile::from_nbt(&v.unwrap()).map(Box::new))
        } else {
            stream(&bytes)
        };
        assert_eq!(
            result.is_ok(),
            row["accepted"].as_bool().unwrap(),
            "{}: {}",
            row["case"],
            result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_else(|| "accepted".to_owned())
        );
        if let Ok(value) = result {
            let mut expected = row["fields"].clone();
            expected["properties"]
                .as_array_mut()
                .unwrap()
                .sort_by_key(|v| units(&v["key"]));
            assert_eq!(fields(&value), expected, "{}", row["case"]);
            if row["canonical_wire_equal"] == true {
                let wire = hex::decode(row["canonical_wire_hex"].as_str().unwrap()).unwrap();
                assert_eq!(*value, *stream(&wire).unwrap(), "{} canonical", row["case"]);
            }
            accepted += 1;
            values.push(Some(value));
        } else {
            values.push(None);
        }
    }
    assert_eq!((values.len(), accepted), (324, 253));
    for pair in facts["pairs"].as_array().unwrap() {
        let a = values[pair["a"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        let b = values[pair["b"].as_u64().unwrap() as usize]
            .as_ref()
            .unwrap();
        assert_eq!(a == b, pair["equal"].as_bool().unwrap(), "{pair}");
    }
    assert_eq!(facts["pairs"].as_array().unwrap().len(), 32131);
}
#[test]
fn original_profile_rules_and_sources_are_bound() {
    use sha2::{Digest, Sha256};
    let source: Json = serde_json::from_str(include_str!(
        "../../../../data/client_api/profile_source.json"
    ))
    .unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for group in ["generators_sha256", "files_sha256"] {
        for (path, hash) in source[group].as_object().unwrap() {
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(root.join(path)).unwrap())
                ),
                hash.as_str().unwrap(),
                "{path}"
            );
        }
    }
    assert_eq!(
        (
            source["cases"].as_u64(),
            source["accepted"].as_u64(),
            source["pairs"].as_u64()
        ),
        (Some(324), Some(253), Some(32131))
    );
}
