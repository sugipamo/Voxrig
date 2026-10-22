//! Frozen common map parser from origin/main 9810cbd; differential oracle only.
use super::*;
fn boolean(r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>) -> Result<bool> {
    // Original ByteBuf.readBoolean accepts any nonzero byte.
    Ok(r.u8()? != 0)
}
pub(crate) fn decode(bytes: &[u8], modern: bool) -> Result<MapUpdate> {
    if bytes.len() > MAX_ICON_BYTES + CELLS + 65_536 {
        bail!("map packet exceeds bounded payload limit");
    }
    let mut r = crate::versions::java_1_21_11::ScoreboardReader::new(bytes);
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
                Some(if modern {
                    let bytes = r.encoded_nbt()?;
                    name_bytes += bytes.len();
                    UiText::NativeNbt { bytes }
                } else {
                    let json = r.string()?;
                    name_bytes += json.len();
                    UiText::LegacyJson { json }
                })
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
