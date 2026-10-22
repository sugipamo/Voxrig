//! Original native parser, retained only as a differential regression oracle.
use super::*;
use crate::versions::java_1_16_1::protocol::{get_string, get_varint};
use anyhow::Context;
pub(crate) fn parse_map_update(payload: &[u8]) -> Result<MapUpdate> {
    let mut rest = payload;
    let id = get_varint(&mut rest)?;
    let scale = take_i8(&mut rest)?;
    let tracking_position = take_bool(&mut rest)?;
    let locked = take_bool(&mut rest)?;
    let icon_count = get_varint(&mut rest)?;
    if !(0..=4096).contains(&icon_count) {
        bail!("invalid map icon count {icon_count}");
    }
    let mut icons = Vec::with_capacity(icon_count as usize);
    for _ in 0..icon_count {
        let kind = get_varint(&mut rest)?;
        let x = take_i8(&mut rest)?;
        let z = take_i8(&mut rest)?;
        let direction = take_u8(&mut rest)?;
        let display_name_json = if take_bool(&mut rest)? {
            Some(get_string(&mut rest)?)
        } else {
            None
        };
        icons.push(MapIcon {
            kind,
            x,
            z,
            direction,
            display_name_json,
        });
    }
    let width = take_u8(&mut rest)?;
    let rectangle = if width == 0 {
        None
    } else {
        let height = take_u8(&mut rest)?;
        let x = take_u8(&mut rest)?;
        let y = take_u8(&mut rest)?;
        if usize::from(x) + usize::from(width) > 128 || usize::from(y) + usize::from(height) > 128 {
            bail!("map rectangle is outside 128x128 bounds");
        }
        let length = get_varint(&mut rest)?;
        let expected = usize::from(width) * usize::from(height);
        if length < 0 || length as usize != expected || rest.len() < expected {
            bail!("invalid map color data length {length}, expected {expected}");
        }
        let colors = rest[..expected].to_vec();
        Some(MapRectangle {
            x,
            y,
            width,
            height,
            colors,
        })
    };
    Ok(MapUpdate {
        id,
        scale,
        tracking_position,
        locked,
        icons,
        rectangle,
    })
}

fn take_u8(rest: &mut &[u8]) -> Result<u8> {
    let value = *rest.first().context("truncated map packet")?;
    *rest = &rest[1..];
    Ok(value)
}

fn take_i8(rest: &mut &[u8]) -> Result<i8> {
    Ok(take_u8(rest)? as i8)
}
fn take_bool(rest: &mut &[u8]) -> Result<bool> {
    Ok(take_u8(rest)? != 0)
}
