//! Structured map-item updates and retained 128×128 color buffers.

use anyhow::{Result, bail};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `MapIcon`.
pub struct MapIcon {
    /// The `kind` value.
    pub kind: i32,
    /// The `x` value.
    pub x: i8,
    /// The `z` value.
    pub z: i8,
    /// The `direction` value.
    pub direction: u8,
    /// The `display_name_json` value.
    pub display_name_json: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `MapData`.
pub struct MapData {
    /// The `id` value.
    pub id: i32,
    /// The `scale` value.
    pub scale: i8,
    /// The `tracking_position` value.
    pub tracking_position: bool,
    /// The `locked` value.
    pub locked: bool,
    /// The `icons` value.
    pub icons: Vec<MapIcon>,
    /// 128×128 vanilla map colors, indexed as `y * 128 + x`.
    pub colors: Arc<[u8; 16_384]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `MapUpdate`.
pub struct MapUpdate {
    /// The `id` value.
    pub id: i32,
    /// The `scale` value.
    pub scale: i8,
    /// The `tracking_position` value.
    pub tracking_position: bool,
    /// The `locked` value.
    pub locked: bool,
    /// The `icons` value.
    pub icons: Vec<MapIcon>,
    /// The `rectangle` value.
    pub rectangle: Option<MapRectangle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `MapRectangle`.
pub struct MapRectangle {
    /// The `x` value.
    pub x: u8,
    /// The `y` value.
    pub y: u8,
    /// The `width` value.
    pub width: u8,
    /// The `height` value.
    pub height: u8,
    /// The `colors` value.
    pub colors: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// State and protocol data represented by `MapStore`.
pub struct MapStore {
    /// The `maps` value.
    pub maps: HashMap<i32, MapData>,
}

impl MapStore {
    pub(crate) fn apply(&mut self, update: &MapUpdate, max_maps: usize) -> Result<()> {
        if !self.maps.contains_key(&update.id) && self.maps.len() >= max_maps {
            bail!("map cache limit of {max_maps} exceeded");
        }
        let map = self.maps.entry(update.id).or_insert_with(|| MapData {
            id: update.id,
            scale: update.scale,
            tracking_position: update.tracking_position,
            locked: update.locked,
            icons: Vec::new(),
            colors: Arc::new([0; 16_384]),
        });
        map.scale = update.scale;
        map.tracking_position = update.tracking_position;
        map.locked = update.locked;
        map.icons = update.icons.clone();
        if let Some(rectangle) = &update.rectangle {
            let colors = Arc::make_mut(&mut map.colors);
            for row in 0..usize::from(rectangle.height) {
                let source = row * usize::from(rectangle.width);
                let destination = (usize::from(rectangle.y) + row) * 128 + usize::from(rectangle.x);
                colors[destination..destination + usize::from(rectangle.width)].copy_from_slice(
                    &rectangle.colors[source..source + usize::from(rectangle.width)],
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod original_decode;
#[cfg(test)]
pub(crate) use original_decode::parse_map_update;

pub(crate) fn project_map_update(update: crate::versions::map_decode::MapUpdate) -> MapUpdate {
    use crate::client::{maps::MapIconKind, ui::UiText};
    MapUpdate {
        id: update.id,
        scale: update.scale,
        tracking_position: update.tracking.expect("legacy tracking flag"),
        locked: update.locked,
        icons: update
            .icons
            .expect("legacy icon list")
            .iter()
            .map(|icon| MapIcon {
                kind: match icon.kind {
                    MapIconKind::LegacyOrdinal(kind) => kind,
                    _ => unreachable!("legacy map kind"),
                },
                x: icon.x,
                z: icon.y,
                direction: icon.encoded_rotation,
                display_name_json: icon.name.as_ref().map(|name| match name {
                    UiText::LegacyJson { json } => json.clone(),
                    _ => unreachable!("legacy map name"),
                }),
            })
            .collect(),
        rectangle: update.patch.map(|patch| MapRectangle {
            x: patch.x,
            y: patch.y,
            width: patch.width,
            height: patch.height,
            colors: patch.colors,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::java_1_16_1::protocol::put_varint;

    #[test]
    fn partial_map_updates_are_applied_without_copying_unchanged_snapshots() {
        let mut packet = Vec::new();
        put_varint(&mut packet, 7);
        packet.extend([2, 1, 0]);
        put_varint(&mut packet, 0);
        packet.extend([2, 1, 3, 4]);
        put_varint(&mut packet, 2);
        packet.extend([9, 10]);
        let update = parse_map_update(&packet).unwrap();
        let mut store = MapStore::default();
        store.apply(&update, 128).unwrap();
        let map = &store.maps[&7];
        assert_eq!(map.colors[4 * 128 + 3], 9);
        assert_eq!(map.colors[4 * 128 + 4], 10);
    }

    #[test]
    fn map_limit_rejects_new_map_without_partial_update() {
        let mut packet = Vec::new();
        put_varint(&mut packet, 7);
        packet.extend([2, 1, 0]);
        put_varint(&mut packet, 0);
        packet.extend([0, 0, 0, 0]);
        let update = parse_map_update(&packet).unwrap();

        let mut store = MapStore::default();
        store.apply(&update, 1).unwrap();
        let before = store.maps.get(&7).cloned().unwrap();

        let mut second = update;
        second.id = 8;
        assert!(store.apply(&second, 1).is_err());
        assert_eq!(store.maps.len(), 1);
        assert_eq!(store.maps.get(&7), Some(&before));
        assert!(!store.maps.contains_key(&8));
    }
}
