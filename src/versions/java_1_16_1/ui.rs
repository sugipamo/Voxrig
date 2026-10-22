//! Boss bars, scoreboard, teams, titles, tab list, and world-border state.

use anyhow::{Result, bail};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
/// State and protocol data represented by `BossBar`.
pub struct BossBar {
    /// The `uuid` value.
    pub uuid: [u8; 16],
    /// The `title_json` value.
    pub title_json: String,
    /// The `health` value.
    pub health: f32,
    /// The `color` value.
    pub color: i32,
    /// The `divisions` value.
    pub divisions: i32,
    /// The `flags` value.
    pub flags: u8,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `ScoreboardObjective`.
pub struct ScoreboardObjective {
    /// The `name` value.
    pub name: String,
    /// The `display_json` value.
    pub display_json: String,
    /// The `render_type` value.
    pub render_type: i32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `Team`.
pub struct Team {
    /// The `name` value.
    pub name: String,
    /// The `display_json` value.
    pub display_json: String,
    /// The `friendly_flags` value.
    pub friendly_flags: i8,
    /// The `name_tag_visibility` value.
    pub name_tag_visibility: String,
    /// The `collision_rule` value.
    pub collision_rule: String,
    /// The `color` value.
    pub color: i32,
    /// The `prefix_json` value.
    pub prefix_json: String,
    /// The `suffix_json` value.
    pub suffix_json: String,
    /// The `members` value.
    pub members: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
/// State and protocol data represented by `TitleState`.
pub struct TitleState {
    /// The `title_json` value.
    pub title_json: Option<String>,
    /// The `subtitle_json` value.
    pub subtitle_json: Option<String>,
    /// The `action_bar_json` value.
    pub action_bar_json: Option<String>,
    /// The `fade_in` value.
    pub fade_in: i32,
    /// The `stay` value.
    pub stay: i32,
    /// The `fade_out` value.
    pub fade_out: i32,
}
impl Default for TitleState {
    fn default() -> Self {
        Self {
            title_json: None,
            subtitle_json: None,
            action_bar_json: None,
            fade_in: 10,
            stay: 70,
            fade_out: 20,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
/// State and protocol data represented by `WorldBorder`.
pub struct WorldBorder {
    /// The `center_x` value.
    pub center_x: f64,
    /// The `center_z` value.
    pub center_z: f64,
    /// The `old_diameter` value.
    pub old_diameter: f64,
    /// The `diameter` value.
    pub diameter: f64,
    /// The `transition_millis` value.
    pub transition_millis: i64,
    /// The `portal_boundary` value.
    pub portal_boundary: i32,
    /// The `warning_time` value.
    pub warning_time: i32,
    /// The `warning_blocks` value.
    pub warning_blocks: i32,
}
#[derive(Clone, Debug, Default, PartialEq)]
/// State and protocol data represented by `UiState`.
pub struct UiState {
    /// The `boss_bars` value.
    pub boss_bars: HashMap<[u8; 16], BossBar>,
    /// The `objectives` value.
    pub objectives: HashMap<String, ScoreboardObjective>,
    /// The `display_objectives` value.
    pub display_objectives: HashMap<i8, String>,
    /// The `scores` value.
    pub scores: HashMap<(String, String), i32>,
    /// The `teams` value.
    pub teams: HashMap<String, Team>,
    /// The `title` value.
    pub title: TitleState,
    /// The `tab_header_json` value.
    pub tab_header_json: String,
    /// The `tab_footer_json` value.
    pub tab_footer_json: String,
    /// The `world_border` value.
    pub world_border: WorldBorder,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Possible values represented by `UiUpdateKind`.
pub enum UiUpdateKind {
    /// The `BossBar` variant.
    BossBar,
    /// The `Objective` variant.
    Objective,
    /// The `DisplayObjective` variant.
    DisplayObjective,
    /// The `Score` variant.
    Score,
    /// The `Team` variant.
    Team,
    /// The `Title` variant.
    Title,
    /// The `TabList` variant.
    TabList,
    /// The `WorldBorder` variant.
    WorldBorder,
}

impl UiState {
    pub(crate) fn project_team(
        &mut self,
        packet: crate::versions::ui_decode::TeamPacket,
    ) -> Result<()> {
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

    pub(crate) fn project_scoreboard(
        &mut self,
        update: crate::versions::ui_decode::ScoreboardUpdate,
    ) {
        use crate::client::ui::ScoreboardRenderType;
        use crate::versions::ui_decode::ScoreboardUpdate as U;
        match update {
            U::Objective(o) => {
                self.objectives.insert(
                    o.name.clone(),
                    ScoreboardObjective {
                        name: o.name,
                        display_json: legacy_json(o.display),
                        render_type: match o.render_type {
                            ScoreboardRenderType::Integer => 0,
                            ScoreboardRenderType::Hearts => 1,
                        },
                    },
                );
            }
            U::RemoveObjective(name) => {
                self.objectives.remove(&name);
                self.display_objectives.retain(|_, v| v != &name);
                self.scores.retain(|(_, objective), _| objective != &name);
            }
            U::Display(slot, name) => {
                if name.is_empty() {
                    self.display_objectives.remove(&(slot as i8));
                } else {
                    self.display_objectives.insert(slot as i8, name);
                }
            }
            U::Score(score) => {
                self.scores
                    .insert((score.owner, score.objective), score.value);
            }
            U::Reset { owner, objective } => {
                if let Some(objective) = objective {
                    self.scores.remove(&(owner, objective));
                } else {
                    self.scores.retain(|(name, _), _| name != &owner);
                }
            }
        }
    }
    pub(crate) fn project_boss_bar(
        &mut self,
        uuid: [u8; 16],
        update: crate::versions::ui_decode::BossBarUpdate,
    ) {
        use crate::versions::ui_decode::BossBarUpdate as U;
        match update {
            U::Add {
                title,
                progress,
                color,
                overlay,
                flags,
            } => {
                self.boss_bars.insert(
                    uuid,
                    BossBar {
                        uuid,
                        title_json: legacy_json(title),
                        health: progress,
                        color: color as i32,
                        divisions: overlay as i32,
                        flags: flags.raw,
                    },
                );
            }
            U::Remove => {
                self.boss_bars.remove(&uuid);
            }
            change => {
                if let Some(bar) = self.boss_bars.get_mut(&uuid) {
                    match change {
                        U::Progress(value) => bar.health = value,
                        U::Title(value) => bar.title_json = legacy_json(value),
                        U::Style(color, overlay) => {
                            bar.color = color as i32;
                            bar.divisions = overlay as i32;
                        }
                        U::Flags(value) => bar.flags = value.raw,
                        U::Add { .. } | U::Remove => unreachable!(),
                    }
                }
            }
        }
    }
    pub(crate) fn project_display(&mut self, update: crate::versions::ui_decode::DisplayUpdate) {
        use crate::client::ui::{TitleTiming, WorldBorderDuration, WorldBorderSize};
        use crate::versions::ui_decode::DisplayUpdate as U;
        let size = |border: &mut WorldBorder, value| match value {
            WorldBorderSize::Set { diameter } => border.diameter = diameter,
            WorldBorderSize::Lerp {
                from_diameter,
                to_diameter,
                duration,
            }
            | WorldBorderSize::Initialize {
                from_diameter,
                to_diameter,
                duration,
            } => {
                border.old_diameter = from_diameter;
                border.diameter = to_diameter;
                border.transition_millis = match duration {
                    WorldBorderDuration::Milliseconds(value) => value,
                    _ => unreachable!("legacy border duration"),
                };
            }
        };
        match update {
            U::Title(text) => self.title.title_json = Some(legacy_json(text)),
            U::Subtitle(text) => self.title.subtitle_json = Some(legacy_json(text)),
            U::ActionBar(text) => self.title.action_bar_json = Some(legacy_json(text)),
            U::Timing(TitleTiming::Set {
                fade_in,
                stay,
                fade_out,
            }) => {
                self.title.fade_in = fade_in;
                self.title.stay = stay;
                self.title.fade_out = fade_out;
            }
            U::Timing(TitleTiming::ResetToDefaults) => {
                unreachable!("wire timing is always explicit")
            }
            U::Clear(reset) => {
                self.title.title_json = None;
                self.title.subtitle_json = None;
                if reset {
                    self.title.fade_in = 10;
                    self.title.stay = 70;
                    self.title.fade_out = 20;
                }
            }
            U::Tab(pair) => {
                self.tab_header_json = legacy_json(pair.header);
                self.tab_footer_json = legacy_json(pair.footer);
            }
            U::Center([x, z]) => {
                self.world_border.center_x = x;
                self.world_border.center_z = z;
            }
            U::Size(value) => size(&mut self.world_border, value),
            U::Delay(value) => self.world_border.warning_time = value,
            U::Distance(value) => self.world_border.warning_blocks = value,
            U::Initialize {
                center: [x, z],
                size: value,
                absolute,
                delay,
                distance,
            } => {
                self.world_border.center_x = x;
                self.world_border.center_z = z;
                size(&mut self.world_border, value);
                self.world_border.portal_boundary = absolute;
                self.world_border.warning_time = delay;
                self.world_border.warning_blocks = distance;
            }
        }
    }
}
fn legacy_json(text: crate::client::ui::UiText) -> String {
    match text {
        crate::client::ui::UiText::LegacyJson { json } => json,
        _ => unreachable!("legacy decoder text"),
    }
}

#[cfg(test)]
mod original_decode;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    use crate::versions::java_1_16_1::protocol::{put_string, put_varint};

    #[test]
    fn scoreboard_create_score_and_remove_are_stateful() {
        let mut state = UiState::default();
        let mut objective = Vec::new();
        put_string(&mut objective, "kills");
        objective.push(0);
        put_string(&mut objective, r#"{"text":"Kills"}"#);
        put_varint(&mut objective, 0);
        state.apply_objective(&objective).unwrap();
        let mut score = Vec::new();
        put_string(&mut score, "Alice");
        put_varint(&mut score, 0);
        put_string(&mut score, "kills");
        put_varint(&mut score, 3);
        state.apply_score(&score).unwrap();
        assert_eq!(
            state.scores.get(&("Alice".into(), "kills".into())),
            Some(&3)
        );
        let mut remove = Vec::new();
        put_string(&mut remove, "kills");
        remove.push(1);
        state.apply_objective(&remove).unwrap();
        assert!(state.objectives.is_empty());
        assert!(state.scores.is_empty());
    }

    #[test]
    fn boss_bar_incremental_updates_preserve_other_fields() {
        let mut state = UiState::default();
        let mut add = vec![7; 16];
        put_varint(&mut add, 0);
        put_string(&mut add, r#"{"text":"Boss"}"#);
        add.extend(0.5f32.to_be_bytes());
        put_varint(&mut add, 2);
        put_varint(&mut add, 4);
        add.push(1);
        state.apply_boss_bar(&add).unwrap();
        let mut health = vec![7; 16];
        put_varint(&mut health, 2);
        health.extend(0.25f32.to_be_bytes());
        state.apply_boss_bar(&health).unwrap();
        let bar = &state.boss_bars[&[7; 16]];
        assert_eq!(bar.health, 0.25);
        assert_eq!(bar.color, 2);
    }
}
