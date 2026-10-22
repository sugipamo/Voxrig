//! Modern UI text framing, retaining each original unnamed NBT field.
use crate::Result;
use crate::client::ui::UiText;
use crate::versions::ui_decode::UiReader;

impl<'a> UiReader<'a> for super::wire::Reader<'a> {
    const MODERN: bool = true;
    fn new(payload: &'a [u8]) -> Self {
        Self::new(payload)
    }
    fn text(&mut self) -> Result<UiText> {
        Ok(UiText::NativeNbt {
            bytes: self.encoded_nbt()?,
        })
    }
}
