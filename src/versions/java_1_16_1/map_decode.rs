//! Version-specific map layout and text policy.
use crate::client::ui::UiText;
use crate::versions::map_decode::{self, MapReader, MapUpdate};
use anyhow::Result;

pub(crate) fn decode_map(bytes: &[u8]) -> Result<MapUpdate> {
    map_decode::decode::<super::ui_decode::Reader<'_>>(bytes)
}
impl<'a> MapReader<'a> for super::ui_decode::Reader<'a> {
    const MODERN: bool = false;
    fn new(bytes: &'a [u8]) -> Self {
        <Self as crate::versions::ui_decode::UiReader>::new(bytes)
    }
    fn name(&mut self) -> Result<UiText> {
        Ok(UiText::LegacyJson {
            json: self.string()?,
        })
    }
}
