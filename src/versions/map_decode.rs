//! Private map wire updates. Adapters select layouts; the common ledger owns retention.
use crate::client::maps::{MapIconKind, ReceivedMapIcon};
use crate::client::ui::UiText;
use anyhow::{Result, bail};
use std::sync::Arc;
const CELLS: usize = 128 * 128;
const MAX_ICON_BYTES: usize = 1_048_576;

pub(super) trait MapReader<'a>:
    std::ops::DerefMut<Target = super::fields::Reader<'a>> + Sized
{
    const MODERN: bool;
    fn new(bytes: &'a [u8]) -> Self;
    fn name(&mut self) -> Result<UiText>;
}
#[derive(Clone)]
pub(crate) struct Patch {
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
    pub colors: Vec<u8>,
}
#[derive(Clone)]
pub(crate) struct MapUpdate {
    pub id: i32,
    pub scale: i8,
    pub locked: bool,
    pub tracking: Option<bool>,
    pub icons: Option<Arc<[ReceivedMapIcon]>>,
    pub patch: Option<Patch>,
}
fn boolean<'a>(r: &mut impl MapReader<'a>) -> Result<bool> {
    // Original ByteBuf.readBoolean accepts any nonzero byte.
    Ok(r.u8()? != 0)
}
pub(super) fn decode<'a, R: MapReader<'a>>(bytes: &'a [u8]) -> Result<MapUpdate> {
    let modern = R::MODERN;
    if bytes.len() > MAX_ICON_BYTES + CELLS + 65_536 {
        bail!("map packet exceeds bounded payload limit");
    }
    let mut r = R::new(bytes);
    let id = r.varint()?;
    let scale = r.u8()? as i8;
    let tracking = if modern { None } else { Some(boolean(&mut r)?) };
    let locked = boolean(&mut r)?;
    let icons = if !modern || boolean(&mut r)? {
        let count = r.count(4096)?;
        let mut icons = Vec::with_capacity(count);
        let mut name_bytes = 0usize;
        for _ in 0..count {
            let kind = r.varint()?;
            if kind < 0 || !modern && kind > 26 {
                bail!("invalid map decoration type");
            }
            let x = r.u8()? as i8;
            let y = r.u8()? as i8;
            let encoded_rotation = r.u8()?;
            let name = if boolean(&mut r)? {
                let name = r.name()?;
                name_bytes += match &name {
                    UiText::NativeNbt { bytes } => bytes.len(),
                    UiText::LegacyJson { json } => json.len(),
                    UiText::Unavailable => unreachable!("decoded map name"),
                };
                Some(name)
            } else {
                None
            };
            if name_bytes > MAX_ICON_BYTES {
                bail!("map icon text storage limit exceeded");
            }
            icons.push(ReceivedMapIcon {
                kind: if modern {
                    MapIconKind::ModernRegistryId(kind)
                } else {
                    MapIconKind::LegacyOrdinal(kind)
                },
                x,
                y,
                encoded_rotation,
                name,
            });
        }
        Some(icons.into())
    } else {
        None
    };
    let width = r.u8()?;
    let patch = if width == 0 {
        None
    } else {
        let height = r.u8()?;
        let x = r.u8()?;
        let y = r.u8()?;
        if usize::from(x) + usize::from(width) > 128 || usize::from(y) + usize::from(height) > 128 {
            bail!("map patch is outside 128 by 128 cells");
        }
        let colors = r.byte_array(CELLS)?.to_vec();
        if colors.len() != usize::from(width) * usize::from(height) {
            bail!("map patch has incorrect color length");
        }
        Some(Patch {
            x,
            y,
            width,
            height,
            colors,
        })
    };
    r.end()?;
    Ok(MapUpdate {
        id,
        scale,
        locked,
        tracking,
        icons,
        patch,
    })
}

#[cfg(test)]
pub(super) mod original_decode;
