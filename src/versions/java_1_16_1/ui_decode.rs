//! Legacy UI text framing over neutral bounded primitives.
use crate::Result;
use crate::client::ui::UiText;
use crate::versions::{fields, ui_decode::UiReader};

pub(crate) struct Reader<'a>(fields::Reader<'a>);
impl<'a> std::ops::Deref for Reader<'a> {
    type Target = fields::Reader<'a>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Reader<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<'a> UiReader<'a> for Reader<'a> {
    const MODERN: bool = false;
    fn new(payload: &'a [u8]) -> Self {
        Self(fields::Reader::new(payload))
    }
    fn text(&mut self) -> Result<UiText> {
        Ok(UiText::LegacyJson {
            json: self.string()?,
        })
    }
}
