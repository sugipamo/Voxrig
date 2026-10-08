//! Fully decoded native spatial receipts, committed to the common spawn lifetime.
use crate::client::entity::NativeMotion;
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
    fn take(&mut self, count: usize) -> anyhow::Result<&'a [u8]> {
        anyhow::ensure!(self.0.len() >= count, "truncated entity motion field");
        let (field, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(field)
    }
    fn varint(&mut self) -> anyhow::Result<i32> {
        crate::protocol::get_varint(&mut self.0)
    }
    fn u8(&mut self) -> anyhow::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn bool(&mut self) -> anyhow::Result<bool> {
        let flag = self.u8()?;
        anyhow::ensure!(flag <= 1, "invalid ground flag");
        Ok(flag == 1)
    }
    fn f64(&mut self) -> anyhow::Result<f64> {
        let value = f64::from_be_bytes(self.take(8)?.try_into()?);
        anyhow::ensure!(
            value.is_finite() && value.abs() <= 33_554_432.0,
            "invalid entity coordinate"
        );
        Ok(value)
    }
    fn f32(&mut self) -> anyhow::Result<f32> {
        let value = f32::from_be_bytes(self.take(4)?.try_into()?);
        anyhow::ensure!(value.is_finite(), "invalid entity rotation");
        Ok(value)
    }
    fn end(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.is_empty(), "trailing entity motion bytes");
        Ok(())
    }
}

pub(super) fn decode(
    id: i32,
    payload: &[u8],
) -> anyhow::Result<Option<(Option<i32>, NativeMotion)>> {
    if !matches!(id, 0x28 | 0x29 | 0x2a | 0x2c | 0x3b | 0x46 | 0x56) {
        return Ok(None);
    }
    let mut r = Reader::new(payload);
    let target = if id == 0x2c { None } else { Some(r.varint()?) };
    anyhow::ensure!(target.is_none_or(|id| id >= 0), "invalid entity motion ID");
    let update = match id {
        0x28 | 0x29 => {
            let delta = [short(&mut r)?, short(&mut r)?, short(&mut r)?];
            let rotation = if id == 0x29 {
                Some([angle(&mut r)?, angle(&mut r)?])
            } else {
                None
            };
            NativeMotion::Relative {
                delta,
                rotation,
                ground: r.bool()?,
            }
        }
        0x2a => NativeMotion::Rotation {
            rotation: [angle(&mut r)?, angle(&mut r)?],
            ground: r.bool()?,
        },
        0x2c | 0x56 => {
            let position = [r.f64()?, r.f64()?, r.f64()?];
            let rotation = if id == 0x2c {
                [r.f32()?, r.f32()?]
            } else {
                [angle(&mut r)?, angle(&mut r)?]
            };
            let ground = if id == 0x56 { Some(r.bool()?) } else { None };
            NativeMotion::Absolute {
                position,
                rotation,
                velocity: None,
                ground,
                reset_base: id == 0x56,
            }
        }
        0x3b => NativeMotion::HeadYaw(angle(&mut r)?),
        0x46 => NativeMotion::Velocity([
            f64::from(short(&mut r)?) / 8000.0,
            f64::from(short(&mut r)?) / 8000.0,
            f64::from(short(&mut r)?) / 8000.0,
        ]),
        _ => unreachable!(),
    };
    r.end()?;
    Ok(Some((target, update)))
}
fn short(r: &mut Reader<'_>) -> anyhow::Result<i16> {
    Ok(i16::from_be_bytes(r.take(2)?.try_into()?))
}
fn angle(r: &mut Reader<'_>) -> anyhow::Result<f32> {
    Ok(f32::from(r.u8()? as i8) * 360.0 / 256.0)
}
