//! Version-specific map layout and text policy.
use crate::client::ui::UiText;
use crate::versions::map_decode::{self, MapReader, MapUpdate};
use anyhow::Result;

pub(crate) fn decode_map(bytes: &[u8]) -> Result<MapUpdate> {
    map_decode::decode::<super::wire::Reader<'_>>(bytes)
}
impl<'a> MapReader<'a> for super::wire::Reader<'a> {
    const MODERN: bool = true;
    fn new(bytes: &'a [u8]) -> Self {
        Self::new(bytes)
    }
    fn name(&mut self) -> Result<UiText> {
        Ok(UiText::NativeNbt {
            bytes: self.encoded_nbt()?,
        })
    }
}
