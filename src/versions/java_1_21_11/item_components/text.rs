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

pub(super) fn project(value: &Arc<NbtValue>) -> Result<Text> {
    let mut budget = 65_536;
    read(value, &mut budget, 0)
}
fn read(value: &Arc<NbtValue>, budget: &mut usize, depth: usize) -> Result<Text> {
    if depth > 64 {
        bail!("text constructor depth limit");
    }
    *budget = budget
        .checked_sub(1)
        .context("text constructor work limit")?;
    match &**value {
        NbtValue::String(text) => Ok(Text {
            contents: Contents::Literal { text: text.clone() },
            style: Style::default(),
            siblings: Vec::new(),
        }),
        NbtValue::List(values) => {
            let (first, rest) = values
                .split_first()
                .context("native text list must be nonempty")?;
            let mut result = read(first, budget, depth + 1)?;
            for value in rest {
                result.siblings.push(read(value, budget, depth + 1)?);
            }
            Ok(result)
        }
        NbtValue::Compound(fields) => {
            let explicit = optional_string(fields, "type")?
                .map(NbtString::text)
                .transpose()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let kind = match explicit.as_deref() {
                Some(
                    "text" | "translatable" | "keybind" | "score" | "selector" | "nbt" | "object",
                ) => explicit.as_deref().unwrap(),
                Some(_) => bail!("unknown native text contents type; update Voxrig"),
                None => [
                    "text",
                    "translate",
                    "keybind",
                    "score",
                    "selector",
                    "nbt",
                    "sprite",
                    "player",
                ]
                .into_iter()
                .find(|key| fields.get(key).is_some())
                .context("native text contents absent")?,
            };
            let contents = match kind {
                "text" => Contents::Literal {
                    text: string(fields, "text")?.clone(),
                },
                "translatable" | "translate" => {
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
                                let value = read(value, budget, depth + 1)?;
                                match &value.contents {
                                    Contents::Literal { text }
                                        if empty_style(&value.style)
                                            && value.siblings.is_empty() =>
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
                        fallback: optional_string(fields, "fallback")?.cloned(),
                        arguments,
                    }
                }
                "keybind" => Contents::Keybind {
                    keybind: string(fields, "keybind")?.clone(),
                },
                "selector" => Contents::Selector {
                    pattern: string(fields, "selector")?.clone(),
                    separator: strict_separator(fields, budget, depth)?,
                },
                "score" => {
                    let fields = fields
                        .get("score")
                        .and_then(NbtValue::as_compound)
                        .context("native score contents must be a compound")?;
                    Contents::Score {
                        name: string(fields, "name")?.clone(),
                        objective: string(fields, "objective")?.clone(),
                    }
                }
                "nbt" => {
                    let source = if fields.get("block").is_some() {
                        NbtSource::Block(string(fields, "block")?.clone())
                    } else if fields.get("entity").is_some() {
                        NbtSource::Entity(string(fields, "entity")?.clone())
                    } else {
                        NbtSource::Storage(identifier(string(fields, "storage")?)?)
                    };
                    Contents::Nbt {
                        path: string(fields, "nbt")?.clone(),
                        interpret: fields.get("interpret").and_then(boolean).unwrap_or(false),
                        separator: separator(fields, budget, depth),
                        source,
                    }
                }
                "object" | "sprite" | "player" => {
                    if let Some(profile) = fields
                        .entries()
                        .iter()
                        .find(|f| f.key().utf16().iter().copied().eq("player".encode_utf16()))
                    {
                        Contents::PlayerSprite {
                            profile: Arc::new(profile.value().clone()),
                            hat: fields.get("hat").and_then(boolean).unwrap_or(true),
                        }
                    } else {
                        Contents::Sprite {
                            atlas: optional_string(fields, "atlas")?
                                .map(identifier)
                                .transpose()?
                                .unwrap_or(Identifier::parse("minecraft:blocks")?),
                            sprite: identifier(string(fields, "sprite")?)?,
                        }
                    }
                }
                _ => unreachable!(),
            };
            let style = style(fields, budget, depth)?;
            let mut siblings = Vec::new();
            if let Some(extra) = fields.get("extra") {
                let values = extra
                    .as_list()
                    .context("native text extra must be a list")?;
                if values.is_empty() {
                    bail!("native extra list must be nonempty");
                }
                for value in values {
                    siblings.push(read(value, budget, depth + 1)?);
                }
            }
            Ok(Text {
                contents,
                style,
                siblings,
            })
        }
        _ => bail!("native text root must be a string, list or compound"),
    }
}
fn separator(fields: &NbtCompound, budget: &mut usize, depth: usize) -> Option<Box<Text>> {
    // The original NBT contents optional separator decoder is lenient.
    // Failed attempts still consume work; they do not get a fresh recursion budget.
    fields
        .entries()
        .iter()
        .find(|f| {
            f.key()
                .utf16()
                .iter()
                .copied()
                .eq("separator".encode_utf16())
        })
        .and_then(|f| read(&Arc::new(f.value().clone()), budget, depth + 1).ok())
        .map(Box::new)
}
fn strict_separator(
    fields: &NbtCompound,
    budget: &mut usize,
    depth: usize,
) -> Result<Option<Box<Text>>> {
    fields
        .get("separator")
        .map(|value| read(&Arc::new(value.clone()), budget, depth + 1).map(Box::new))
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
fn integer(value: &NbtValue) -> Option<i32> {
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
fn style(fields: &NbtCompound, budget: &mut usize, depth: usize) -> Result<Style> {
    let flag = |name| {
        fields
            .get(name)
            .map(|v| boolean(v).with_context(|| format!("invalid native style flag {name}")))
            .transpose()
    };
    let color = optional_string(fields, "color")?.map(color).transpose()?;
    let shadow_color = fields.get("shadow_color").map(shadow).transpose()?;
    let click = fields.get("click_event").map(click).transpose()?;
    let hover = fields
        .get("hover_event")
        .map(|v| hover(v, budget, depth))
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
fn click(value: &NbtValue) -> Result<Click> {
    let fields = value
        .as_compound()
        .context("native click event must be a compound")?;
    let kind = string(fields, "action")?
        .text()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(match kind.as_str() {
        "open_url" => Click::OpenUrl(string(fields, "url")?.clone()),
        "open_file" => Click::OpenFile(string(fields, "path")?.clone()),
        "run_command" => Click::RunCommand(string(fields, "command")?.clone()),
        "suggest_command" => Click::SuggestCommand(string(fields, "command")?.clone()),
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
        "show_dialog" => Click::Dialog(Arc::new(
            fields
                .get("dialog")
                .context("native dialog required")?
                .clone(),
        )),
        _ => bail!("unknown native click action; update Voxrig"),
    })
}
fn hover(value: &NbtValue, budget: &mut usize, depth: usize) -> Result<Hover> {
    let fields = value
        .as_compound()
        .context("native hover event must be a compound")?;
    let kind = string(fields, "action")?
        .text()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(match kind.as_str() {
        "show_text" => {
            let value = fields.get("value").context("native hover text required")?;
            Hover::Text(Box::new(read(&Arc::new(value.clone()), budget, depth + 1)?))
        }
        "show_item" => Hover::Item(Arc::new(value.clone())),
        "show_entity" => Hover::Entity(Arc::new(value.clone())),
        _ => bail!("unknown native hover action; update Voxrig"),
    })
}
