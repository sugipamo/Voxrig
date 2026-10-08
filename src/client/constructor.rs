//! Limits are separate from native constructor rejection so fallback cannot hide them.
#[derive(Debug)]
pub(crate) struct Limit(pub &'static str);
impl std::fmt::Display for Limit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for Limit {}

/// Original CHAT_STRING fields retain UTF-16 and validate each Java char.
/// Clipboard/literal fields use different codecs and do not use this grammar.
pub(crate) fn chat_string(
    value: &super::nbt::NbtString,
    budget: &mut usize,
) -> anyhow::Result<super::nbt::NbtString> {
    use serde::Deserialize;
    use std::sync::OnceLock;
    #[derive(Deserialize)]
    struct Rules {
        chat_excluded_utf16: Vec<u16>,
    }
    static RULES: OnceLock<Rules> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../data/client_api/click_constructor_rules-1.21.11.json"
        ))
        .expect("pinned original chat constructor grammar")
    });
    *budget = budget
        .checked_sub(value.utf16().len())
        .ok_or(Limit("chat constructor work limit"))?;
    if value
        .utf16()
        .iter()
        .any(|u| rules.chat_excluded_utf16.binary_search(u).is_ok())
    {
        anyhow::bail!("invalid native chat constructor character");
    }
    Ok(value.clone())
}

/// Missing semantic support is never a proven native constructor rejection.
#[derive(Debug)]
pub(crate) struct Unresolved(pub &'static str);
impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Unresolved {}
pub(crate) fn preserve(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Limit>().is_some() || error.downcast_ref::<Unresolved>().is_some()
}
