//! Bounded last-received map item fields; no inferred pixels or world position.
use super::{ObservedValue, SessionStamp, received, ui::UiText};
use anyhow::{Result, bail};
use std::{collections::BTreeMap, sync::Arc};

/// Maximum number of map identities retained in one common world context.
pub const MAX_RECEIVED_MAPS: usize = 128;
const CELLS: usize = 128 * 128;
const MAX_ICON_BYTES: usize = 1_048_576;

/// Original world-scoped map observation identity, distinct from an item slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct MapIdentity {
    /// Connection/version/world in which the packet was received.
    pub session: SessionStamp,
    /// Original native map ID; not a location or registry ID.
    pub native_id: i32,
    /// First receipt in this world context.
    pub first_sequence: u64,
}
/// Original icon type. Modern IDs refer to the version's decoration registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum MapIconKind {
    /// Original legacy enum ordinal.
    LegacyOrdinal(i32),
    /// Original modern registry holder ID. No static name fallback.
    ModernRegistryId(i32),
}
/// Received map icon, without conversion to block/world coordinates.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedMapIcon {
    /// Version-bound original type.
    pub kind: MapIconKind,
    /// Signed native map X.
    pub x: i8,
    /// Signed native map Y (not world Y).
    pub y: i8,
    /// Original rotation byte; native rotation is its low four bits.
    pub encoded_rotation: u8,
    /// Original optional component, kept in its native representation.
    pub name: Option<UiText>,
}
impl ReceivedMapIcon {
    /// Native normalized rotation in sixteenths of a turn.
    pub fn rotation(&self) -> u8 {
        self.encoded_rotation & 15
    }
}
/// Immutable received map fields and partial pixel coverage.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MapObservation {
    /// Original identity. Maps do not encode their owning dimension or center.
    pub map: MapIdentity,
    /// Capture boundary; individual sources retain original packet sequences.
    pub receive_sequence: u64,
    /// Last original scale byte, without inference from pixels.
    pub scale: ObservedValue<i8>,
    /// Last original locked flag.
    pub locked: ObservedValue<bool>,
    /// Legacy-only explicit tracking-position flag; modern is None.
    pub tracking_position: Option<ObservedValue<bool>>,
    /// Last explicit icon list. Modern omission preserves the previous receipt;
    /// an explicit empty list replaces it. None means never received.
    pub icons: Option<ObservedValue<Arc<[ReceivedMapIcon]>>>,
    #[serde(skip)]
    colors: Arc<[u8]>,
    #[serde(skip)]
    sources: Arc<[u64]>,
}
impl MapObservation {
    /// Last explicitly received color and its original source. Missing coverage
    /// is None, including after a first partial patch. Known color zero is Some.
    pub fn pixel(&self, x: u8, y: u8) -> Option<ObservedValue<u8>> {
        if x >= 128 || y >= 128 {
            return None;
        }
        let cell = usize::from(y) * 128 + usize::from(x);
        let sequence = self.sources[cell];
        (sequence != 0).then(|| received(self.colors[cell], sequence))
    }
    /// Number of cells explicitly covered by received patches.
    pub fn known_pixels(&self) -> usize {
        self.sources.iter().filter(|s| **s != 0).count()
    }
}
impl crate::Client {
    /// Read one bounded map capture, including after closure. World replacement
    /// retires the cache; saved captures keep their original world and coverage.
    pub async fn map_observation(&self, native_id: i32) -> crate::Result<Option<MapObservation>> {
        super::dispatch!(&self.adapter,a=>super::adapter::CoreOps::map_observation(a,native_id).await)
    }
}

#[derive(Clone, Default)]
pub(crate) struct MapLedger(BTreeMap<i32, StoredMap>);
#[derive(Clone)]
struct StoredMap {
    first_sequence: u64,
    scale: ObservedValue<i8>,
    locked: ObservedValue<bool>,
    tracking_position: Option<ObservedValue<bool>>,
    icons: Option<ObservedValue<Arc<[ReceivedMapIcon]>>>,
    colors: Arc<[u8]>,
    sources: Arc<[u64]>,
}
struct Patch {
    x: u8,
    y: u8,
    width: u8,
    height: u8,
    colors: Vec<u8>,
}
struct Update {
    id: i32,
    scale: i8,
    locked: bool,
    tracking: Option<bool>,
    icons: Option<Arc<[ReceivedMapIcon]>>,
    patch: Option<Patch>,
}
fn boolean(r: &mut crate::versions::java_1_21_11::ScoreboardReader<'_>) -> Result<bool> {
    // Original ByteBuf.readBoolean accepts any nonzero byte.
    Ok(r.u8()? != 0)
}
fn decode(bytes: &[u8], modern: bool) -> Result<Update> {
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
    Ok(Update {
        id,
        scale,
        locked,
        tracking,
        icons,
        patch,
    })
}
impl MapLedger {
    pub(crate) fn receive(&mut self, bytes: &[u8], modern: bool, sequence: u64) -> Result<()> {
        self.receive_with_limit(bytes, modern, sequence, MAX_RECEIVED_MAPS)
    }
    pub(crate) fn receive_with_limit(
        &mut self,
        bytes: &[u8],
        modern: bool,
        sequence: u64,
        maximum: usize,
    ) -> Result<()> {
        let update = decode(bytes, modern)?;
        if sequence == 0 {
            bail!("map update has no original receipt sequence");
        }
        if !self.0.contains_key(&update.id) && self.0.len() >= maximum {
            bail!("common map cache limit exceeded");
        }
        let map = self.0.entry(update.id).or_insert_with(|| StoredMap {
            first_sequence: sequence,
            scale: received(update.scale, sequence),
            locked: received(update.locked, sequence),
            tracking_position: None,
            icons: None,
            colors: vec![0; CELLS].into(),
            sources: vec![0; CELLS].into(),
        });
        map.scale = received(update.scale, sequence);
        map.locked = received(update.locked, sequence);
        map.tracking_position = update.tracking.map(|v| received(v, sequence));
        if let Some(icons) = update.icons {
            map.icons = Some(received(icons, sequence));
        }
        if let Some(patch) = update.patch {
            let colors = Arc::make_mut(&mut map.colors);
            let sources = Arc::make_mut(&mut map.sources);
            for row in 0..usize::from(patch.height) {
                let start = (usize::from(patch.y) + row) * 128 + usize::from(patch.x);
                let end = start + usize::from(patch.width);
                let input = row * usize::from(patch.width);
                colors[start..end]
                    .copy_from_slice(&patch.colors[input..input + usize::from(patch.width)]);
                sources[start..end].fill(sequence);
            }
        }
        Ok(())
    }
    pub(crate) fn capture(
        &self,
        id: i32,
        session: SessionStamp,
        sequence: u64,
    ) -> Option<MapObservation> {
        self.0.get(&id).map(|map| MapObservation {
            map: MapIdentity {
                session,
                native_id: id,
                first_sequence: map.first_sequence,
            },
            receive_sequence: sequence,
            scale: map.scale.clone(),
            locked: map.locked.clone(),
            tracking_position: map.tracking_position.clone(),
            icons: map.icons.clone(),
            colors: map.colors.clone(),
            sources: map.sources.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(world_generation: u64) -> SessionStamp {
        SessionStamp {
            version: crate::MinecraftVersion::Java1_21_11,
            connection_id: 71,
            world_generation,
        }
    }
    fn patch(modern: bool, colors: &[u8]) -> Vec<u8> {
        let mut bytes = vec![7, 0, 0];
        if !modern {
            bytes.push(0);
        }
        bytes.push(0); // omitted modern icons / explicit empty legacy list
        bytes.extend([2, 1, 3, 4, colors.len() as u8]);
        bytes.extend(colors);
        bytes
    }
    #[test]
    fn maps_partial_coverage_omission_atomicity_and_saved_identity() {
        for modern in [false, true] {
            let mut ledger = MapLedger::default();
            assert!(ledger.capture(7, session(1), 1).is_none());
            let bytes = patch(modern, &[0, 19]);
            ledger.receive(&bytes, modern, 8).unwrap();
            let first = ledger.capture(7, session(1), 8).unwrap();
            assert_eq!(first.known_pixels(), 2);
            assert_eq!(first.pixel(3, 4).unwrap().value, 0);
            assert!(first.pixel(2, 4).is_none());
            assert!(first.pixel(128, 4).is_none());
            assert_eq!(first.icons.is_none(), modern);
            let mut update = patch(modern, &[31, 0]);
            update[if modern { 6 } else { 7 }] = 4; // overlap one old pixel
            ledger.receive(&update, modern, 12).unwrap();
            let next = ledger.capture(7, session(1), 15).unwrap();
            assert_eq!(next.known_pixels(), 3);
            assert_eq!(
                next.pixel(3, 4).unwrap().source,
                super::super::ValueSource::Received { sequence: 8 }
            );
            assert_eq!(
                next.pixel(4, 4).unwrap().source,
                super::super::ValueSource::Received { sequence: 12 }
            );
            assert_eq!(first.pixel(4, 4).unwrap().value, 19);
            for bad in [
                &bytes[..bytes.len() - 1],
                &[bytes.as_slice(), &[0]].concat(),
            ] {
                assert!(ledger.receive(bad, modern, 18).is_err());
            }
            assert_eq!(
                ledger.capture(7, session(1), 19).unwrap().scale.source,
                super::super::ValueSource::Received { sequence: 12 }
            );
            ledger = MapLedger::default();
            assert!(ledger.capture(7, session(2), 20).is_none());
            ledger.receive(&bytes, modern, 22).unwrap();
            assert_ne!(ledger.capture(7, session(2), 22).unwrap().map, first.map);
            assert_eq!(first.map.session, session(1));
        }
    }
    #[test]
    fn maps_explicit_icons_replace_omission_preserves_and_budget_is_atomic() {
        let mut ledger = MapLedger::default();
        let icons = [7, 0, 0, 1, 1, 0, 255, 127, 255, 0, 0];
        ledger.receive(&icons, true, 2).unwrap();
        let first = ledger.capture(7, session(1), 2).unwrap();
        let icon = &first.icons.as_ref().unwrap().value[0];
        assert_eq!(
            (icon.x, icon.y, icon.rotation(), icon.encoded_rotation),
            (-1, 127, 15, 255)
        );
        ledger.receive(&[7, 0, 1, 0, 0], true, 3).unwrap();
        assert_eq!(
            ledger
                .capture(7, session(1), 3)
                .unwrap()
                .icons
                .unwrap()
                .source,
            super::super::ValueSource::Received { sequence: 2 }
        );
        ledger.receive(&[7, 0, 1, 1, 0, 0], true, 4).unwrap();
        assert!(
            ledger
                .capture(7, session(1), 4)
                .unwrap()
                .icons
                .unwrap()
                .value
                .is_empty()
        );
        for id in 0..MAX_RECEIVED_MAPS {
            let mut bytes = Vec::new();
            crate::protocol::put_varint(&mut bytes, id as i32);
            bytes.extend([0, 0, 0, 0]);
            ledger.receive(&bytes, true, 5).unwrap();
        }
        let mut bytes = Vec::new();
        crate::protocol::put_varint(&mut bytes, 999);
        bytes.extend([0, 0, 0, 0]);
        assert!(ledger.receive(&bytes, true, 6).is_err());
        assert!(ledger.capture(999, session(1), 6).is_none());
        assert_eq!(
            ledger.capture(7, session(1), 6).unwrap().scale.source,
            super::super::ValueSource::Received { sequence: 5 }
        );
    }
    #[test]
    fn maps_native_names_geometry_and_configured_native_limit() {
        for modern in [false, true] {
            let mut bytes = vec![7, 0, 0];
            if !modern {
                bytes.push(0);
            }
            if modern {
                bytes.push(1);
            }
            bytes.extend([1, 0, 255, 127, 255, 1]);
            let name = if modern {
                vec![8, 0, 3, b'm', b'a', b'p']
            } else {
                let mut v = Vec::new();
                crate::protocol::put_string(&mut v, "{\"text\":\"map\"}");
                v
            };
            bytes.extend(&name);
            bytes.push(0);
            let mut ledger = MapLedger::default();
            ledger.receive(&bytes, modern, 1).unwrap();
            let first = ledger.capture(7, session(1), 1).unwrap();
            let icon = &first.icons.as_ref().unwrap().value[0];
            assert!(if modern {
                matches!(&icon.name,Some(UiText::NativeNbt { bytes }) if bytes==&name)
            } else {
                matches!(&icon.name,Some(UiText::LegacyJson {json}) if json=="{\"text\":\"map\"}")
            });
            let mut invalid = patch(modern, &[0, 0]);
            invalid[if modern { 6 } else { 7 }] = 127;
            assert!(ledger.receive(&invalid, modern, 2).is_err());
            assert!(
                ledger
                    .receive(&bytes[..bytes.len() - 2], modern, 2)
                    .is_err()
            );
            assert!(ledger.receive(&bytes, modern, 0).is_err());
            assert_eq!(
                ledger.capture(7, session(1), 3).unwrap().scale.source,
                super::super::ValueSource::Received { sequence: 1 }
            );
        }
        let mut ledger = MapLedger::default();
        for id in 0..130 {
            let mut bytes = Vec::new();
            crate::protocol::put_varint(&mut bytes, id);
            bytes.extend([0, 0, 0, 0]);
            ledger
                .receive_with_limit(&bytes, true, id as u64 + 1, 130)
                .unwrap();
        }
        assert!(ledger.capture(129, session(1), 130).is_some());
    }
}
