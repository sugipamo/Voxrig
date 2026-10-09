//! Native text field construction from logical NBT. Complex references remain
//! explicit dependencies; this is not complete constructor validation/equality.
use crate::client::{
    nbt::{NbtCompound, NbtString, NbtValue},
    text::*,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
#[derive(serde::Deserialize)]
struct ColorRules {
    colors: BTreeMap<String, u32>,
    hex_utf16_digits: BTreeMap<u16, i32>,
}
fn color_rules() -> &'static ColorRules {
    static RULES: OnceLock<ColorRules> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../data/client_api/text_color_rules-1.21.11.json"
        ))
        .expect("pinned native text color grammar")
    })
}

use crate::client::constructor::Limit as ReadLimit;

pub(super) fn project(value: &Arc<NbtValue>) -> Result<Text> {
    let mut budget = 65_536;
    read(value, &mut budget, 0)
}
pub(super) trait Resolver {
    fn charge(&mut self) -> Result<()>;
    fn item(&mut self, value: &NbtValue, depth: usize) -> Result<Hover>;
    fn dialog(&mut self, value: &NbtValue, depth: usize) -> Result<Click>;
}
type Dependencies<'a> = Option<&'a mut dyn Resolver>;
pub(super) fn project_bound(value: &Arc<NbtValue>, resolver: &mut dyn Resolver) -> Result<Text> {
    read_inner(value, &mut 65_536, 0, &mut Some(resolver))
}
fn read(value: &Arc<NbtValue>, budget: &mut usize, depth: usize) -> Result<Text> {
    read_inner(value, budget, depth, &mut None)
}
fn read_inner(
    value: &Arc<NbtValue>,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Text> {
    if let Some(context) = resolver.as_deref_mut() {
        context.charge()?;
    }

    if depth > 64 {
        return Err(ReadLimit("text constructor depth limit").into());
    }
    *budget = budget
        .checked_sub(1)
        .ok_or(ReadLimit("text constructor work limit"))?;
    match &**value {
        NbtValue::String(text) => Ok(Text {
            source: Some(value.clone()),
            contents: Contents::Literal { text: text.clone() },
            style: Style::default(),
            siblings: Vec::new(),
        }),
        NbtValue::List(values) => {
            let (first, rest) = values
                .split_first()
                .context("native text list must be nonempty")?;
            let mut result = read_inner(first, budget, depth + 1, resolver)?;
            result.source = Some(value.clone());
            for value in rest {
                result
                    .siblings
                    .push(read_inner(value, budget, depth + 1, resolver)?);
            }
            Ok(result)
        }
        NbtValue::Compound(fields) => {
            let contents = dispatch(fields, "type", &constructor_rules().contents, |kind| {
                read_contents(fields, kind, budget, depth, resolver)
            })?;
            let style = style(fields, budget, depth, resolver)?;
            let mut siblings = Vec::new();
            if let Some(extra) = fields.get("extra") {
                let values = extra
                    .as_list()
                    .context("native text extra must be a list")?;
                if values.is_empty() {
                    bail!("native extra list must be nonempty");
                }
                for value in values {
                    siblings.push(read_inner(value, budget, depth + 1, resolver)?);
                }
            }
            Ok(Text {
                source: Some(value.clone()),
                contents,
                style,
                siblings,
            })
        }
        _ => bail!("native text root must be a string, list or compound"),
    }
}
#[derive(serde::Deserialize)]
struct ConstructorRules {
    contents: Vec<String>,
    sources: Vec<String>,
    objects: Vec<String>,
}
fn constructor_rules() -> &'static ConstructorRules {
    static RULES: OnceLock<ConstructorRules> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../data/client_api/text_constructor_rules-1.21.11.json"
        ))
        .expect("pinned original native constructor order")
    })
}
fn dispatch<T>(
    fields: &NbtCompound,
    discriminator: &str,
    kinds: &[String],
    mut decode: impl FnMut(&str) -> Result<T>,
) -> Result<T> {
    // StrictEither never tries fuzzy fallback when a discriminator is present.
    if fields.get(discriminator).is_some() {
        let kind = string(fields, discriminator)?
            .text()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if !kinds.contains(&kind) {
            bail!("unknown native text {discriminator}; update Voxrig");
        }
        return decode(&kind);
    }
    // Original FuzzyCodec accepts the first successful decoder in mapper order,
    // not the first present key. Failed attempts keep the same work budget.
    for kind in kinds {
        match decode(kind) {
            Ok(value) => return Ok(value),
            Err(error) if crate::client::constructor::preserve(&error) => return Err(error),
            Err(_) => {}
        }
    }
    bail!("no matching native text {discriminator} constructor")
}
#[inline(never)]
fn read_contents(
    fields: &NbtCompound,
    kind: &str,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Contents> {
    Ok(match kind {
        "text" => Contents::Literal {
            text: string(fields, "text")?.clone(),
        },
        "translatable" => {
            let mut arguments = Vec::new();
            if let Some(values) = fields.get("with") {
                for value in values
                    .as_list()
                    .context("native translation arguments must be a list")?
                {
                    if let Some(number) = argument_number(value) {
                        arguments.push(Argument::Number(number));
                    } else if let NbtValue::String(value) = &**value {
                        arguments.push(Argument::String(value.clone()));
                    } else {
                        let value = read_inner(value, budget, depth + 1, resolver)?;
                        match &value.contents {
                            Contents::Literal { text }
                                if empty_style(&value.style) && value.siblings.is_empty() =>
                            {
                                arguments.push(Argument::String(text.clone()))
                            }
                            _ => arguments.push(Argument::Text(Box::new(value))),
                        }
                    }
                }
            }
            Contents::Translate {
                key: string(fields, "translate")?.clone(),
                fallback: fields
                    .get("fallback")
                    .and_then(NbtValue::as_string)
                    .cloned(),
                arguments,
            }
        }
        "keybind" => Contents::Keybind {
            keybind: string(fields, "keybind")?.clone(),
        },
        "selector" => Contents::Selector {
            pattern: crate::client::selector::parse(string(fields, "selector")?, budget)?.0,
            separator: strict_separator(fields, budget, depth, resolver)?,
        },
        "score" => {
            let fields = fields
                .get("score")
                .and_then(NbtValue::as_compound)
                .context("native score contents must be a compound")?;
            Contents::Score {
                name: match crate::client::selector::parse(string(fields, "name")?, budget) {
                    Ok((pattern, _)) => crate::client::text::ScoreName::Selector(pattern),
                    Err(error) if crate::client::constructor::preserve(&error) => {
                        return Err(error);
                    }
                    Err(_) => {
                        crate::client::text::ScoreName::Literal(string(fields, "name")?.clone())
                    }
                },
                objective: string(fields, "objective")?.clone(),
            }
        }
        "nbt" => {
            let source = dispatch(fields, "source", &constructor_rules().sources, |kind| {
                Ok(match kind {
                    "entity" => NbtSource::Entity(string(fields, "entity")?.clone()),
                    "block" => NbtSource::Block(string(fields, "block")?.clone()),
                    "storage" => NbtSource::Storage(identifier(string(fields, "storage")?)?),
                    _ => bail!("unknown native text data source; update Voxrig"),
                })
            })?;
            Contents::Nbt {
                path: string(fields, "nbt")?.clone(),
                interpret: fields.get("interpret").and_then(boolean).unwrap_or(false),
                separator: separator(fields, budget, depth, resolver)?,
                source,
            }
        }
        "object" => dispatch(fields, "object", &constructor_rules().objects, |kind| {
            Ok(match kind {
                "atlas" => Contents::Sprite {
                    atlas: optional_string(fields, "atlas")?
                        .map(identifier)
                        .transpose()?
                        .unwrap_or(Identifier::parse("minecraft:blocks")?),
                    sprite: identifier(string(fields, "sprite")?)?,
                },
                "player" => Contents::PlayerSprite {
                    profile: Box::new(crate::client::profile::from_nbt(
                        fields
                            .get("player")
                            .context("native player profile required")?,
                    )?),
                    hat: fields
                        .get("hat")
                        .map(|value| boolean(value).context("invalid native player hat flag"))
                        .transpose()?
                        .unwrap_or(true),
                },
                _ => bail!("unknown native text object; update Voxrig"),
            })
        })?,
        _ => bail!("unknown native text contents; update Voxrig"),
    })
}
fn separator(
    fields: &NbtCompound,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Option<Box<Text>>> {
    let Some(value) = fields.get("separator") else {
        return Ok(None);
    };
    // Native lenient optional ignores semantic decode errors. Our resource limit
    // is not such an error: never silently discard a valid, expensive separator.
    match read_inner(&Arc::new(value.clone()), budget, depth + 1, resolver) {
        Ok(value) => Ok(Some(Box::new(value))),
        Err(error) if crate::client::constructor::preserve(&error) => Err(error),
        Err(_) => Ok(None),
    }
}
fn strict_separator(
    fields: &NbtCompound,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Option<Box<Text>>> {
    fields
        .get("separator")
        .map(|value| {
            read_inner(&Arc::new(value.clone()), budget, depth + 1, resolver).map(Box::new)
        })
        .transpose()
}
fn argument_number(value: &NbtValue) -> Option<Number> {
    Some(match value {
        NbtValue::Byte(v) => Number::Byte(*v),
        NbtValue::Short(v) => Number::Short(*v),
        NbtValue::Int(v) => Number::Integer(*v),
        NbtValue::Long(v) => Number::Long(*v),
        NbtValue::Float { bits } => Number::Float { bits: *bits },
        NbtValue::Double { bits } => Number::Double { bits: *bits },
        _ => return None,
    })
}
fn string<'a>(fields: &'a NbtCompound, name: &str) -> Result<&'a NbtString> {
    fields
        .get(name)
        .and_then(NbtValue::as_string)
        .with_context(|| format!("native text string field {name} required"))
}
fn optional_string<'a>(fields: &'a NbtCompound, name: &str) -> Result<Option<&'a NbtString>> {
    fields
        .get(name)
        .map(|value| {
            value
                .as_string()
                .with_context(|| format!("native text string field {name} invalid"))
        })
        .transpose()
}
fn identifier(value: &NbtString) -> Result<Identifier> {
    Identifier::parse(&value.text().map_err(|e| anyhow::anyhow!(e.to_string()))?)
}
fn float_number(value: &NbtValue) -> Option<f32> {
    Some(match value {
        NbtValue::Byte(value) => f32::from(*value),
        NbtValue::Short(value) => f32::from(*value),
        NbtValue::Int(value) => *value as f32,
        NbtValue::Long(value) => *value as f32,
        NbtValue::Float { bits } => f32::from_bits(*bits),
        NbtValue::Double { bits } => f64::from_bits(*bits) as f32,
        _ => return None,
    })
}
pub(super) fn integer(value: &NbtValue) -> Option<i32> {
    Some(match value {
        NbtValue::Byte(v) => i32::from(*v),
        NbtValue::Short(v) => i32::from(*v),
        NbtValue::Int(v) => *v,
        NbtValue::Long(v) => *v as i32,
        NbtValue::Float { bits } => f32::from_bits(*bits) as i32,
        NbtValue::Double { bits } => f64::from_bits(*bits) as i32,
        _ => return None,
    })
}
fn boolean(value: &NbtValue) -> Option<bool> {
    integer(value).map(|n| n as i8 != 0)
}
fn style(
    fields: &NbtCompound,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Style> {
    let flag = |name| {
        fields
            .get(name)
            .map(|v| boolean(v).with_context(|| format!("invalid native style flag {name}")))
            .transpose()
    };
    let color = optional_string(fields, "color")?.map(color).transpose()?;
    let shadow_color = fields.get("shadow_color").map(shadow).transpose()?;
    let click = fields
        .get("click_event")
        .map(|v| click(v, budget, depth, resolver))
        .transpose()?;
    let hover = fields
        .get("hover_event")
        .map(|v| hover(v, budget, depth, resolver))
        .transpose()?;
    Ok(Style {
        color,
        shadow_color,
        bold: flag("bold")?,
        italic: flag("italic")?,
        underlined: flag("underlined")?,
        strikethrough: flag("strikethrough")?,
        obfuscated: flag("obfuscated")?,
        click,
        hover,
        insertion: optional_string(fields, "insertion")?.cloned(),
        font: optional_string(fields, "font")?
            .map(identifier)
            .transpose()?,
    })
}
fn empty_style(style: &Style) -> bool {
    style.color.is_none()
        && style.shadow_color.is_none()
        && style.bold.is_none()
        && style.italic.is_none()
        && style.underlined.is_none()
        && style.strikethrough.is_none()
        && style.obfuscated.is_none()
        && style.click.is_none()
        && style.hover.is_none()
        && style.insertion.is_none()
        && style.font.is_none()
}
fn color(value: &NbtString) -> Result<Color> {
    let spelling = value.text().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if let Some(hex) = spelling.strip_prefix('#') {
        // The original uses Integer.parseInt(radix16), allowing a leading sign,
        // leading zeros and variable width, with an explicit RGB range check.
        let rgb = hexadecimal(hex)?;
        if !(0..=0xffffff).contains(&rgb) {
            bail!("native RGB out of range");
        }
        return Ok(Color {
            rgb: rgb as u32,
            serialized: format!("#{rgb:06X}"),
        });
    }
    let rgb = *color_rules()
        .colors
        .get(&spelling)
        .context("unknown native named text color")?;
    Ok(Color {
        rgb,
        serialized: spelling,
    })
}
fn hexadecimal(value: &str) -> Result<i32> {
    let mut chars = value.encode_utf16().peekable();
    let negative = match chars.peek().copied() {
        Some(45) => {
            chars.next();
            true
        }
        Some(43) => {
            chars.next();
            false
        }
        _ => false,
    };
    if chars.peek().is_none() {
        bail!("empty native RGB integer");
    }
    let limit = if negative { i32::MIN } else { -i32::MAX };
    let mut result = 0i32;
    for c in chars {
        let digit = *color_rules()
            .hex_utf16_digits
            .get(&c)
            .context("invalid native hex digit")?;
        if result < limit / 16 {
            bail!("native RGB integer overflow");
        }
        result *= 16;
        if result < limit + digit {
            bail!("native RGB integer overflow");
        }
        result -= digit;
    }
    Ok(if negative { result } else { -result })
}
fn shadow(value: &NbtValue) -> Result<i32> {
    if let Some(value) = integer(value) {
        return Ok(value);
    }
    let values = value
        .as_list()
        .context("native shadow color must be a number or RGBA list")?;
    if values.len() != 4 {
        bail!("native RGBA requires4 values");
    }
    let mut color = [0u32; 4];
    for (out, value) in color.iter_mut().zip(values) {
        let value = float_number(value).context("native RGBA component must be numeric")?;
        // Original ARGB.as8BitChannel uses Mth.floor(f32 *255), including its
        // saturating cast/wrapping decrement, then color() masks each channel.
        let scaled = value * 255.0;
        let integer = scaled as i32;
        let floor = if scaled < integer as f32 {
            integer.wrapping_sub(1)
        } else {
            integer
        };
        *out = floor as u32 & 255;
    }
    Ok(((color[3] << 24) | (color[0] << 16) | (color[1] << 8) | color[2]) as i32)
}
fn click(
    value: &NbtValue,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Click> {
    let fields = value
        .as_compound()
        .context("native click event must be a compound")?;
    let kind = string(fields, "action")?
        .text()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(match kind.as_str() {
        "open_url" => Click::OpenUrl(Box::new(crate::client::uri::parse(
            string(fields, "url")?,
            budget,
        )?)),
        "open_file" => bail!("native text stream forbids OPEN_FILE click events"),
        "run_command" => Click::RunCommand(crate::client::constructor::chat_string(
            string(fields, "command")?,
            budget,
        )?),
        "suggest_command" => Click::SuggestCommand(crate::client::constructor::chat_string(
            string(fields, "command")?,
            budget,
        )?),
        "copy_to_clipboard" => Click::Copy(string(fields, "value")?.clone()),
        "change_page" => {
            let value = fields
                .get("page")
                .and_then(integer)
                .context("native click page required")?;
            if value < 1 {
                bail!("native click page must be positive");
            }
            Click::ChangePage(value)
        }
        "custom" => Click::Custom {
            id: identifier(string(fields, "id")?)?,
            payload: fields.get("payload").map(|v| Arc::new(v.clone())),
        },
        "show_dialog" => {
            let value = fields.get("dialog").context("native dialog required")?;
            match resolver.as_deref_mut() {
                Some(context) => context.dialog(value, depth)?,
                None => Click::Dialog(Arc::new(value.clone())),
            }
        }
        _ => bail!("unknown native click action; update Voxrig"),
    })
}
fn hover(
    value: &NbtValue,
    budget: &mut usize,
    depth: usize,
    resolver: &mut Dependencies<'_>,
) -> Result<Hover> {
    let fields = value
        .as_compound()
        .context("native hover event must be a compound")?;
    let kind = string(fields, "action")?
        .text()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(match kind.as_str() {
        "show_text" => {
            let value = fields.get("value").context("native hover text required")?;
            Hover::Text(Box::new(read_inner(
                &Arc::new(value.clone()),
                budget,
                depth + 1,
                resolver,
            )?))
        }
        "show_item" => match resolver.as_deref_mut() {
            Some(context) => context.item(value, depth)?,
            None => Hover::Item(Arc::new(value.clone())),
        },
        "show_entity" => {
            #[derive(serde::Deserialize)]
            struct Rules {
                entity_types: Vec<String>,
            }
            static TYPES: OnceLock<Rules> = OnceLock::new();
            let types = TYPES.get_or_init(|| {
                serde_json::from_str(include_str!(
                    "../../../../data/client_api/entity_tooltip_rules-1.21.11.json"
                ))
                .expect("pinned native builtin entity type catalog")
            });
            let entity_type = identifier(string(fields, "id")?)?;
            if !types
                .entity_types
                .contains(&format!("{}:{}", entity_type.namespace, entity_type.path))
            {
                bail!("unknown native entity tooltip type; update Voxrig");
            }
            let value = fields
                .get("uuid")
                .context("native entity tooltip UUID required")?;
            let uuid = match value {
                NbtValue::String(v) => crate::client::uuid::from_string(v.utf16())
                    .context("invalid native entity tooltip UUID string")?,
                _ => crate::client::uuid::from_nbt(value)?,
            };
            let name = fields
                .get("name")
                .map(|v| {
                    read_inner(&Arc::new(v.clone()), budget, depth + 1, resolver).map(Box::new)
                })
                .transpose()?;
            Hover::Entity(Box::new(EntityTooltip {
                entity_type,
                uuid,
                name,
            }))
        }
        _ => bail!("unknown native hover action; update Voxrig"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn text_resource_limits_never_become_fuzzy_fallback_or_missing_separator() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/text_core_cases-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let rows = facts["cases"].as_array().unwrap();
        let separator_row = rows
            .iter()
            .find(|row| {
                row["accepted"] == true
                    && row["fields"]["contents_class"] == "zl"
                    && !row["fields"]["body"]["separator"].is_null()
            })
            .unwrap();
        let root = crate::client::nbt::decode_unnamed_tag(
            &hex::decode(separator_row["input_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap()
        .unwrap();
        let mut budget = 1;
        assert!(
            read(&root, &mut budget, 0)
                .unwrap_err()
                .downcast_ref::<ReadLimit>()
                .is_some()
        );
        let mut budget = 100;
        let value = read(&root, &mut budget, 0).unwrap();
        assert!(matches!(
            value.contents,
            Contents::Nbt {
                separator: Some(_),
                ..
            }
        ));
        assert!(
            read(&root, &mut budget, 65)
                .unwrap_err()
                .downcast_ref::<ReadLimit>()
                .is_some()
        );

        let row = rows
            .iter()
            .find(|row| row["case"] == "translate-styled-arg")
            .unwrap();
        let mut wire = hex::decode(row["input_hex"].as_str().unwrap()).unwrap();
        assert_eq!(wire.pop(), Some(0));
        // A lower-priority valid keybind must not hide resource exhaustion in
        // the original higher-priority translation argument.
        wire.extend_from_slice(&[8, 0, 7]);
        wire.extend_from_slice(b"keybind");
        wire.extend_from_slice(&[0, 8]);
        wire.extend_from_slice(b"key.jump");
        wire.push(0);
        let root = crate::client::nbt::decode_unnamed_tag(&wire)
            .unwrap()
            .unwrap();
        let mut budget = 1;
        assert!(
            read(&root, &mut budget, 0)
                .unwrap_err()
                .downcast_ref::<ReadLimit>()
                .is_some()
        );
        let mut budget = 100;
        assert!(matches!(
            read(&root, &mut budget, 0).unwrap().contents,
            Contents::Translate { .. }
        ));
    }
}

#[cfg(test)]
mod selector_limit_tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn native_valid_selectors_cannot_become_literal_score_or_lower_fuzzy_candidate_on_limit() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/selector_cases-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for kind in ["fuzzy-nbt-", "score-"] {
            let row = facts["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| {
                    row["case"].as_str().unwrap().starts_with(kind)
                        && row["accepted"] == true
                        && (row["fields"]["body"]["pattern"] == "@a"
                            || row["fields"]["body"]["name"] == "@a")
                })
                .unwrap();
            let value = crate::client::nbt::decode_unnamed_tag(
                &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
            )
            .unwrap()
            .unwrap();
            let error = read(&value, &mut 1, 0).unwrap_err();
            assert!(error.downcast_ref::<ReadLimit>().is_some(), "{error}");
            assert!(project(&value).unwrap().modern_field_key().is_some());
        }
    }
}

#[cfg(test)]
mod uri_limit_tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn native_valid_uri_limit_propagates_through_click_and_lenient_nbt_separator() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/uri_cases-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (prefix, nodes) in [("text-direct-", 1usize), ("text-nbt-", 2usize)] {
            let row = facts["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| {
                    row["case"].as_str().unwrap().starts_with(prefix)
                        && row["accepted"] == true
                        && !row["uri_clicks"].as_array().unwrap().is_empty()
                })
                .unwrap();
            let uri = row["uri_clicks"][0]["fields"]["raw"].as_str().unwrap();
            let root = crate::client::nbt::decode_unnamed_tag(
                &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
            )
            .unwrap()
            .unwrap();
            let mut budget = uri.encode_utf16().count() + nodes - 1;
            let error = read(&root, &mut budget, 0).unwrap_err();
            assert!(error.downcast_ref::<ReadLimit>().is_some(), "{error}");
            assert_eq!(error.to_string(), "URI constructor work limit");
            assert!(project(&root).unwrap().modern_field_key().is_some());
        }
    }
}

#[cfg(test)]
mod chat_limit_tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn native_valid_command_limits_cannot_be_hidden_in_lenient_nbt_separator() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/click_constructor_cases-1.21.11.json.gz")
                [..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (case, nodes) in [
            ("run_command-scalar-1", 1usize),
            ("nested-run_command-nbt-0", 2usize),
        ] {
            let row = facts["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["case"] == case)
                .unwrap();
            assert_eq!(row["accepted"], true);
            let command = row["clicks"][0]["fields"]["command"].as_str().unwrap();
            let root = crate::client::nbt::decode_unnamed_tag(
                &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
            )
            .unwrap()
            .unwrap();
            let mut budget = command.encode_utf16().count() + nodes - 1;
            let error = read(&root, &mut budget, 0).unwrap_err();
            assert!(error.downcast_ref::<ReadLimit>().is_some(), "{error}");
            assert_eq!(error.to_string(), "chat constructor work limit");
            assert!(project(&root).unwrap().modern_field_key().is_some());
        }
    }
}

#[cfg(test)]
mod entity_limit_tests {
    use super::*;
    use std::io::Read;
    #[test]
    fn entity_names_propagate_limits_and_nested_unresolved_dependencies() {
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../../data/client_api/entity_tooltip_cases-1.21.11.json.gz")[..],
        )
        .read_to_end(&mut bytes)
        .unwrap();
        let facts: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let row = facts["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["case"] == "name-8")
            .unwrap();
        assert_eq!(row["accepted"], true);
        let root = crate::client::nbt::decode_unnamed_tag(
            &hex::decode(row["input_hex"].as_str().unwrap()).unwrap(),
        )
        .unwrap()
        .unwrap();
        let error = read(&root, &mut 7, 0).unwrap_err();
        assert!(error.downcast_ref::<ReadLimit>().is_some());
        assert_eq!(error.to_string(), "chat constructor work limit");
        let mut value = project(&root).unwrap();
        assert!(value.modern_field_key().is_some());
        // Dependency propagation is a model invariant, separate from the native
        // constructor oracle. An unresolved nested item/dialog cannot become a
        // complete equality key through its outer entity tooltip.
        let Some(Hover::Entity(entity)) = &mut value.style.hover else {
            panic!("native entity tooltip missing")
        };
        let name = entity.name.as_mut().unwrap();
        name.style.hover = Some(Hover::Item(root.clone()));
        name.style.click = Some(Click::Dialog(root));
        assert_eq!(
            value.dependencies(),
            vec![Dependency::Dialog, Dependency::Item]
        );
        assert!(value.modern_field_key().is_none());
    }
}
