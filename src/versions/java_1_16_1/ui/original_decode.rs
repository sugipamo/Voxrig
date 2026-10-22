//! Original native parsers, retained only as differential regression oracles.
use super::*;
use crate::versions::java_1_16_1::protocol::{get_string, get_varint};
use anyhow::Context;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::Cursor;

impl UiState {
    pub(crate) fn apply_boss_bar(&mut self, payload: &[u8]) -> Result<()> {
        let mut rest = payload;
        let uuid = take_uuid(&mut rest)?;
        let action = get_varint(&mut rest)?;
        match action {
            0 => {
                let title_json = get_string(&mut rest)?;
                let health = take_f32(&mut rest)?;
                let color = get_varint(&mut rest)?;
                let divisions = get_varint(&mut rest)?;
                let flags = take_u8(&mut rest)?;
                self.boss_bars.insert(
                    uuid,
                    BossBar {
                        uuid,
                        title_json,
                        health,
                        color,
                        divisions,
                        flags,
                    },
                );
            }
            1 => {
                self.boss_bars.remove(&uuid);
            }
            2 => {
                if let Some(bar) = self.boss_bars.get_mut(&uuid) {
                    bar.health = take_f32(&mut rest)?;
                }
            }
            3 => {
                if let Some(bar) = self.boss_bars.get_mut(&uuid) {
                    bar.title_json = get_string(&mut rest)?;
                }
            }
            4 => {
                let color = get_varint(&mut rest)?;
                let divisions = get_varint(&mut rest)?;
                if let Some(bar) = self.boss_bars.get_mut(&uuid) {
                    bar.color = color;
                    bar.divisions = divisions;
                }
            }
            5 => {
                let flags = take_u8(&mut rest)?;
                if let Some(bar) = self.boss_bars.get_mut(&uuid) {
                    bar.flags = flags;
                }
            }
            _ => bail!("unknown boss bar action {action}"),
        }
        Ok(())
    }
    pub(crate) fn apply_objective(&mut self, payload: &[u8]) -> Result<()> {
        let mut r = payload;
        let name = get_string(&mut r)?;
        let action = take_i8(&mut r)?;
        match action {
            0 | 2 => {
                let display_json = get_string(&mut r)?;
                let render_type = get_varint(&mut r)?;
                self.objectives.insert(
                    name.clone(),
                    ScoreboardObjective {
                        name,
                        display_json,
                        render_type,
                    },
                );
            }
            1 => {
                self.objectives.remove(&name);
                self.display_objectives.retain(|_, v| v != &name);
                self.scores.retain(|(_, objective), _| objective != &name);
            }
            _ => bail!("unknown objective action {action}"),
        }
        Ok(())
    }
    pub(crate) fn apply_display(&mut self, p: &[u8]) -> Result<()> {
        let mut r = p;
        let slot = take_i8(&mut r)?;
        let name = get_string(&mut r)?;
        if name.is_empty() {
            self.display_objectives.remove(&slot);
        } else {
            self.display_objectives.insert(slot, name);
        }
        Ok(())
    }
    pub(crate) fn apply_score(&mut self, p: &[u8]) -> Result<()> {
        let mut r = p;
        let item = get_string(&mut r)?;
        let action = get_varint(&mut r)?;
        let objective = get_string(&mut r)?;
        if action == 1 {
            if objective.is_empty() {
                self.scores.retain(|(owner, _), _| owner != &item);
            } else {
                self.scores.remove(&(item, objective));
            }
        } else {
            let value = get_varint(&mut r)?;
            self.scores.insert((item, objective), value);
        }
        Ok(())
    }
    pub(crate) fn apply_team(&mut self, p: &[u8]) -> Result<()> {
        let packet =
            crate::versions::ui_decode::decode_team(crate::MinecraftVersion::Java1_16_1, p)?;
        let key = packet.name;
        let mode = packet.operation;
        if mode == 0 && self.teams.contains_key(&key) {
            bail!("duplicate native team declaration");
        }
        if mode == 1 {
            self.teams.remove(&key);
            return Ok(());
        }
        if mode != 0 && !self.teams.contains_key(&key) {
            return Ok(());
        }
        if mode == 4
            && packet
                .members
                .iter()
                .any(|p| !self.teams[&key].members.contains(p))
        {
            bail!("team leave does not match native membership");
        }
        if let Some(params) = packet.parameters {
            let json = |text: crate::client::ui::UiText| match text {
                crate::client::ui::UiText::LegacyJson { json } => json,
                _ => unreachable!("legacy team decoder emitted another text format"),
            };
            let members = self
                .teams
                .remove(&key)
                .map_or_else(Vec::new, |team| team.members);
            self.teams.insert(
                key.clone(),
                Team {
                    name: key.clone(),
                    display_json: json(params.display),
                    friendly_flags: params.friendly_flags as i8,
                    name_tag_visibility: params.visibility.legacy_name().to_owned(),
                    collision_rule: params.collision.legacy_name().to_owned(),
                    color: params.color.native_id(),
                    prefix_json: json(params.prefix),
                    suffix_json: json(params.suffix),
                    members,
                },
            );
        }
        if mode == 0 || mode == 3 {
            for member in packet.members {
                for (name, team) in &mut self.teams {
                    if name != &key {
                        team.members.retain(|p| p != &member);
                    }
                }
                let team = self.teams.get_mut(&key).unwrap();
                if !team.members.contains(&member) {
                    team.members.push(member);
                }
            }
        } else if mode == 4 {
            self.teams
                .get_mut(&key)
                .unwrap()
                .members
                .retain(|p| !packet.members.contains(p));
        }
        Ok(())
    }
    pub(crate) fn apply_title(&mut self, p: &[u8]) -> Result<()> {
        let mut r = p;
        let action = get_varint(&mut r)?;
        match action {
            0 => self.title.title_json = Some(get_string(&mut r)?),
            1 => self.title.subtitle_json = Some(get_string(&mut r)?),
            2 => self.title.action_bar_json = Some(get_string(&mut r)?),
            3 => {
                let mut c = Cursor::new(r);
                self.title.fade_in = c.read_i32::<BigEndian>()?;
                self.title.stay = c.read_i32::<BigEndian>()?;
                self.title.fade_out = c.read_i32::<BigEndian>()?;
            }
            4 | 5 => {
                self.title.title_json = None;
                self.title.subtitle_json = None;
                if action == 5 {
                    self.title.fade_in = 10;
                    self.title.stay = 70;
                    self.title.fade_out = 20;
                }
            }
            a => bail!("unknown title action {a}"),
        }
        Ok(())
    }
    pub(crate) fn apply_tab(&mut self, p: &[u8]) -> Result<()> {
        let mut r = p;
        self.tab_header_json = get_string(&mut r)?;
        self.tab_footer_json = get_string(&mut r)?;
        Ok(())
    }
    pub(crate) fn apply_border(&mut self, p: &[u8]) -> Result<()> {
        let mut r = p;
        match get_varint(&mut r)? {
            0 => self.world_border.diameter = take_f64(&mut r)?,
            1 => {
                self.world_border.old_diameter = take_f64(&mut r)?;
                self.world_border.diameter = take_f64(&mut r)?;
                self.world_border.transition_millis = get_varlong(&mut r)?;
            }
            2 => {
                self.world_border.center_x = take_f64(&mut r)?;
                self.world_border.center_z = take_f64(&mut r)?;
            }
            3 => {
                self.world_border.center_x = take_f64(&mut r)?;
                self.world_border.center_z = take_f64(&mut r)?;
                self.world_border.old_diameter = take_f64(&mut r)?;
                self.world_border.diameter = take_f64(&mut r)?;
                self.world_border.transition_millis = get_varlong(&mut r)?;
                self.world_border.portal_boundary = get_varint(&mut r)?;
                self.world_border.warning_blocks = get_varint(&mut r)?;
                self.world_border.warning_time = get_varint(&mut r)?;
            }
            4 => self.world_border.warning_time = get_varint(&mut r)?,
            5 => self.world_border.warning_blocks = get_varint(&mut r)?,
            a => bail!("unknown world border action {a}"),
        }
        Ok(())
    }
}
fn take_u8(r: &mut &[u8]) -> Result<u8> {
    let v = *r.first().context("truncated UI packet")?;
    *r = &r[1..];
    Ok(v)
}
fn take_i8(r: &mut &[u8]) -> Result<i8> {
    Ok(take_u8(r)? as i8)
}
fn take_uuid(r: &mut &[u8]) -> Result<[u8; 16]> {
    if r.len() < 16 {
        bail!("truncated UUID")
    };
    let mut v = [0; 16];
    v.copy_from_slice(&r[..16]);
    *r = &r[16..];
    Ok(v)
}
fn take_f32(r: &mut &[u8]) -> Result<f32> {
    let mut c = Cursor::new(*r);
    let v = c.read_f32::<BigEndian>()?;
    *r = &r[4..];
    Ok(v)
}
fn take_f64(r: &mut &[u8]) -> Result<f64> {
    let mut c = Cursor::new(*r);
    let v = c.read_f64::<BigEndian>()?;
    *r = &r[8..];
    Ok(v)
}
fn get_varlong(r: &mut &[u8]) -> Result<i64> {
    let mut value = 0u64;
    for i in 0..10 {
        let b = take_u8(r)?;
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value as i64);
        }
    }
    bail!("varlong too long")
}
