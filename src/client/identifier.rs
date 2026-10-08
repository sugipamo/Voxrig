//! Original resource-ID grammar shared by the supported native versions.
//! This resolves spelling only, never a registry entry or an observed binding.
use anyhow::{Result, bail};

pub(crate) fn parts(value: &str) -> Result<(&str, &str)> {
    let (namespace, path) = match value.split_once(':') {
        Some(("", path)) => ("minecraft", path),
        Some(parts) => parts,
        None => ("minecraft", value),
    };
    fn allowed(c: u8) -> bool {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
    }
    if !namespace.bytes().all(allowed) || !path.bytes().all(|c| allowed(c) || c == b'/') {
        bail!("invalid native resource identifier");
    }
    // Both original factories accept empty paths. Do not add application-level
    // assumptions (such as a nonempty path or path traversal normalization).
    Ok((namespace, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn both_original_factories_match_ascii_boundaries_aliases_and_utf16_inputs() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../data/client_api/component_normalization_cases-1.21.11.json.gz")
                [..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for version in ["legacy_identifiers", "identifiers"] {
            let rows = facts[version].as_array().unwrap();
            assert_eq!(rows.len(), 274);
            for row in rows {
                let units: Vec<_> = row["input_utf16"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_u64().unwrap() as u16)
                    .collect();
                let text = String::from_utf16(&units);
                let Ok(text) = text else {
                    // Resource IDs reject every surrogate; the modern native
                    // wire string's replacement is also an invalid ID character.
                    assert_eq!(row["accepted"], false);
                    continue;
                };
                let decoded = parts(&text);
                assert_eq!(
                    decoded.is_ok(),
                    row["accepted"].as_bool().unwrap(),
                    "{version}: {text:?}"
                );
                if let Ok((namespace, path)) = decoded {
                    assert_eq!(namespace, row["namespace"]);
                    assert_eq!(path, row["path"]);
                }
            }
        }
    }
}
