//! Modern SelectorPattern validation and raw-pattern equality; no entity resolution.
use super::{constructor::Limit, nbt::NbtString};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::OnceLock};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct Pattern(pub NbtString);
#[derive(Deserialize)]
struct Rules {
    options: BTreeSet<String>,
    entity_types: BTreeSet<String>,
    whitespace_utf16: Vec<u16>,
    unquoted_utf16: Vec<u16>,
    number_utf16: Vec<u16>,
}
fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../data/client_api/selector_rules-1.21.11.json"
        ))
        .expect("pinned original selector grammar")
    })
}
pub(crate) struct Reader<'a> {
    pub units: &'a [u16],
    pub cursor: usize,
    pub budget: &'a mut usize,
}
impl Reader<'_> {
    pub fn peek(&self) -> Option<u16> {
        self.units.get(self.cursor).copied()
    }
    pub fn take(&mut self) -> Result<u16> {
        *self.budget = self
            .budget
            .checked_sub(1)
            .ok_or(Limit("selector constructor work limit"))?;
        let unit = self.peek().context("unexpected end of selector")?;
        self.cursor += 1;
        Ok(unit)
    }
    pub fn consume(&mut self, character: u8) -> Result<bool> {
        if self.peek() == Some(u16::from(character)) {
            self.take()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub fn expect(&mut self, character: u8) -> Result<()> {
        if !self.consume(character)? {
            bail!("expected selector delimiter {}", char::from(character));
        }
        Ok(())
    }
    pub fn space(&mut self) -> Result<()> {
        while self
            .peek()
            .is_some_and(|u| rules().whitespace_utf16.binary_search(&u).is_ok())
        {
            self.take()?;
        }
        Ok(())
    }
    pub fn word(&mut self) -> Result<Vec<u16>> {
        let mut word = Vec::new();
        while self
            .peek()
            .is_some_and(|u| rules().unquoted_utf16.binary_search(&u).is_ok())
        {
            word.push(self.take()?);
        }
        Ok(word)
    }
    pub fn string(&mut self) -> Result<Vec<u16>> {
        let Some(quote @ (34 | 39)) = self.peek() else {
            return self.word();
        };
        self.take()?;
        let mut value = Vec::new();
        loop {
            let unit = self.take()?;
            if unit == quote {
                return Ok(value);
            }
            if unit == 92 {
                let escaped = self.take()?;
                if escaped != quote && escaped != 92 {
                    bail!("invalid Brigadier quoted escape");
                }
                value.push(escaped);
            } else {
                value.push(unit);
            }
        }
    }
    fn number_token(&mut self, range: bool) -> Result<Option<String>> {
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|u| rules().number_utf16.binary_search(&u).is_ok())
        {
            if range && self.units.get(self.cursor..self.cursor + 2) == Some(&[46, 46][..]) {
                break;
            }
            self.take()?;
        }
        if self.cursor == start {
            return Ok(None);
        }
        Ok(Some(String::from_utf16(&self.units[start..self.cursor])?))
    }
    fn number(&mut self, integer: bool, range: bool) -> Result<Option<f64>> {
        self.number_token(range)?
            .map(|s| {
                if integer {
                    Ok(f64::from(s.parse::<i32>()?))
                } else {
                    Ok(s.parse::<f64>()?)
                }
            })
            .transpose()
    }
    fn bounds(&mut self, integer: bool, ordered: bool, positive: bool) -> Result<()> {
        let low = self.number(integer, true)?;
        let high = if self.consume(b'.')? {
            self.expect(b'.')?;
            self.number(integer, true)?
        } else {
            low
        };
        if low.is_none() && high.is_none() {
            bail!("empty selector range");
        }
        if ordered && low.zip(high).is_some_and(|(a, b)| a > b) {
            bail!("inverted selector range");
        }
        if positive && low.into_iter().chain(high).any(|v| v < 0.) {
            bail!("negative selector range");
        }
        Ok(())
    }
    fn identifier(&mut self) -> Result<String> {
        let start = self.cursor;
        while self.peek().is_some_and(|u| {
            u <= 127
                && (u8::try_from(u).unwrap().is_ascii_alphanumeric()
                    || b"_-.:/".contains(&(u as u8)))
        }) {
            self.take()?;
        }
        let raw = String::from_utf16(&self.units[start..self.cursor])?;
        let (namespace, path) = super::identifier::parts(&raw)?;
        Ok(format!("{namespace}:{path}"))
    }
}
#[derive(Default)]
struct State {
    seen: BTreeSet<String>,
    name_equals: bool,
    name_not: bool,
    game_equals: bool,
    game_not: bool,
    team_equals: bool,
    type_limited: bool,
    type_inverse: bool,
    current: bool,
}
fn text(units: Vec<u16>) -> Result<String> {
    Ok(String::from_utf16(&units)?)
}
fn applicable(key: &str, state: &State) -> bool {
    match key {
        "name" => !state.name_equals,
        "gamemode" => !state.game_equals,
        "team" => !state.team_equals,
        "type" => !state.type_limited,
        "limit" | "sort" => !state.current && !state.seen.contains(key),
        "tag" | "nbt" | "predicate" => true,
        _ => !state.seen.contains(key),
    }
}
fn option(r: &mut Reader<'_>, key: &str, state: &mut State) -> Result<()> {
    if !rules().options.contains(key) || !applicable(key, state) {
        bail!("unknown or inapplicable selector option");
    }
    match key {
        "name" => {
            let inverted = r.consume(b'!')?;
            if !inverted && state.name_not {
                bail!("selector name equals after exclusion");
            }
            r.string()?;
            state.name_equals = !inverted;
            state.name_not |= inverted;
        }
        "team" => {
            let inverted = r.consume(b'!')?;
            r.word()?;
            state.team_equals = !inverted;
        }
        "gamemode" => {
            let inverted = r.consume(b'!')?;
            if !inverted && state.game_not {
                bail!("selector gamemode equals after exclusion");
            }
            let mode = text(r.word()?)?;
            if !["survival", "creative", "adventure", "spectator"].contains(&mode.as_str()) {
                bail!("invalid selector gamemode");
            }
            state.game_equals = !inverted;
            state.game_not |= inverted;
        }
        "type" => {
            let inverted = r.consume(b'!')?;
            if !inverted && state.type_inverse {
                bail!("selector type equals after exclusion");
            }
            let tag = r.consume(b'#')?;
            let id = r.identifier()?;
            if !tag && !rules().entity_types.contains(&id) {
                bail!("unknown original selector entity type; update Voxrig");
            }
            state.type_inverse |= inverted;
            state.type_limited = !tag && !inverted;
        }
        "tag" => {
            r.consume(b'!')?;
            r.word()?;
        }
        "predicate" => {
            r.consume(b'!')?;
            r.identifier()?;
        }
        "sort" => {
            if !["nearest", "furthest", "random", "arbitrary"].contains(&text(r.word()?)?.as_str())
            {
                bail!("unknown selector ordering");
            }
        }
        "limit" => {
            if r.number(true, false)?.context("selector limit required")? < 1. {
                bail!("selector limit must be positive");
            }
        }
        "distance" => r.bounds(false, true, true)?,
        "level" => r.bounds(true, true, true)?,
        "x_rotation" | "y_rotation" => r.bounds(false, false, false)?,
        "x" | "y" | "z" | "dx" | "dy" | "dz" => {
            r.number(false, false)?
                .context("selector coordinate required")?;
        }
        "scores" => {
            r.expect(b'{')?;
            r.space()?;
            while r.peek() != Some(125) {
                r.word()?;
                r.space()?;
                r.expect(b'=')?;
                r.space()?;
                r.bounds(true, true, false)?;
                r.space()?;
                if !r.consume(b',')? {
                    break;
                }
                r.space()?;
            }
            r.expect(b'}')?;
        }
        "advancements" => {
            r.expect(b'{')?;
            r.space()?;
            while r.peek() != Some(125) {
                r.identifier()?;
                r.space()?;
                r.expect(b'=')?;
                r.space()?;
                if r.consume(b'{')? {
                    r.space()?;
                    while r.peek() != Some(125) {
                        r.word()?;
                        r.space()?;
                        r.expect(b'=')?;
                        r.space()?;
                        boolean(r)?;
                        r.space()?;
                        if !r.consume(b',')? {
                            break;
                        }
                        r.space()?;
                    }
                    r.expect(b'}')?;
                } else {
                    boolean(r)?;
                }
                r.space()?;
                if !r.consume(b',')? {
                    break;
                }
                r.space()?;
            }
            r.expect(b'}')?;
        }
        "nbt" => {
            r.consume(b'!')?;
            super::selector_snbt::compound(r, 0)?;
        }
        _ => bail!("unimplemented original selector option; update Voxrig"),
    }
    state.seen.insert(key.to_owned());
    Ok(())
}
fn boolean(r: &mut Reader<'_>) -> Result<()> {
    if !["true", "false"].contains(&text(r.word()?)?.as_str()) {
        bail!("selector boolean required");
    }
    Ok(())
}
pub(crate) fn uuid_name(units: &[u16]) -> bool {
    if units.len() > 36 {
        return false;
    }
    #[derive(Deserialize)]
    struct Digits {
        hex_utf16_digits: std::collections::BTreeMap<String, u8>,
    }
    static DIGITS: OnceLock<std::collections::BTreeMap<u16, u8>> = OnceLock::new();
    let digits = DIGITS.get_or_init(|| {
        let facts: Digits = serde_json::from_str(include_str!(
            "../../data/client_api/text_color_rules-1.21.11.json"
        ))
        .expect("pinned JDK hex digit grammar");
        facts
            .hex_utf16_digits
            .into_iter()
            .map(|(u, n)| (u.parse().unwrap(), n))
            .collect()
    });
    let parts: Vec<_> = units.split(|u| *u == 45).collect();
    if parts.len() != 5 {
        return false;
    }
    parts.into_iter().all(|part| {
        let part = part.strip_prefix(&[43]).unwrap_or(part);
        if part.is_empty() {
            return false;
        }
        let mut number = 0i64;
        for unit in part {
            let Some(digit) = digits.get(unit) else {
                return false;
            };
            let Some(next) = number
                .checked_mul(16)
                .and_then(|n| n.checked_add(i64::from(*digit)))
            else {
                return false;
            };
            number = next;
        }
        true
    })
}
pub(crate) fn parse(value: &NbtString, budget: &mut usize) -> Result<(Pattern, usize)> {
    let mut r = Reader {
        units: value.utf16(),
        cursor: 0,
        budget,
    };
    if r.consume(b'@')? {
        let selector = r.take()?;
        if !matches!(selector, 97 | 101 | 110 | 112 | 114 | 115) {
            bail!("unknown selector kind");
        }
        let mut state = State {
            current: selector == 115,
            type_limited: matches!(selector, 112 | 97 | 114),
            ..State::default()
        };
        if r.consume(b'[')? {
            r.space()?;
            while r.peek() != Some(93) {
                r.space()?;
                let key = text(r.string()?)?;
                r.space()?;
                r.expect(b'=')?;
                r.space()?;
                option(&mut r, &key, &mut state)?;
                r.space()?;
                if !r.consume(b',')? {
                    break;
                }
            }
            r.expect(b']')?;
        }
    } else {
        let name = r.string()?;
        if !uuid_name(&name) && (name.is_empty() || name.len() > 16) {
            bail!("invalid selector name or UUID");
        }
    }
    Ok((Pattern(value.clone()), r.cursor))
}
