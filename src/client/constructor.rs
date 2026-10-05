//! Limits are separate from native constructor rejection so fallback cannot hide them.
#[derive(Debug)]
pub(crate) struct Limit(pub &'static str);
impl std::fmt::Display for Limit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for Limit {}
