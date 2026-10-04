//! Shared, internal text constructor fields. Registry/profile/item/dialog/URI
//! dependencies are explicit. A retained dependency is never a native equality
//! result, persistent hash or authority to perform an item operation.
use super::nbt::{NbtString, NbtValue};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Text {
    pub contents: Contents,
    pub style: Style,
    pub siblings: Vec<Text>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Contents {
    Literal {
        text: NbtString,
    },
    Keybind {
        keybind: NbtString,
    },
    Translate {
        key: NbtString,
        fallback: Option<NbtString>,
        arguments: Vec<Argument>,
    },
    Selector {
        pattern: NbtString,
        separator: Option<Box<Text>>,
    },
    Score {
        name: NbtString,
        objective: NbtString,
    },
    Nbt {
        path: NbtString,
        interpret: bool,
        separator: Option<Box<Text>>,
        source: NbtSource,
    },
    Sprite {
        atlas: Identifier,
        sprite: Identifier,
    },
    PlayerSprite {
        profile: Arc<NbtValue>,
        hat: bool,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum Argument {
    String(NbtString),
    Number(Number),
    Text(Box<Text>),
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum Number {
    Byte(i8),
    Short(i16),
    Integer(i32),
    Long(i64),
    Float { bits: u32 },
    Double { bits: u64 },
}
impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        use Number::*;
        match (self, other) {
            (Byte(a), Byte(b)) => a == b,
            (Short(a), Short(b)) => a == b,
            (Integer(a), Integer(b)) => a == b,
            (Long(a), Long(b)) => a == b,
            (Float { bits: a }, Float { bits: b }) => {
                a == b || f32::from_bits(*a).is_nan() && f32::from_bits(*b).is_nan()
            }
            (Double { bits: a }, Double { bits: b }) => {
                a == b || f64::from_bits(*a).is_nan() && f64::from_bits(*b).is_nan()
            }
            _ => false,
        }
    }
}
impl Eq for Number {}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct Identifier {
    pub namespace: String,
    pub path: String,
}
impl Identifier {
    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        let (namespace, path) = super::identifier::parts(value)?;
        Ok(Self {
            namespace: namespace.to_owned(),
            path: path.to_owned(),
        })
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum NbtSource {
    Block(NbtString),
    Entity(NbtString),
    Storage(Identifier),
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct FieldKey {
    contents: ContentsKey,
    style: StyleKey,
    siblings: Vec<FieldKey>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
enum ContentsKey {
    Literal(NbtString),
    Keybind(NbtString),
    Translate {
        key: NbtString,
        fallback: Option<NbtString>,
        arguments: Vec<ArgumentKey>,
    },
    Nbt {
        path: NbtString,
        interpret: bool,
        separator: Option<Box<FieldKey>>,
        source: NbtSource,
    },
    Sprite(Identifier, Identifier),
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
enum ArgumentKey {
    String(NbtString),
    Number(Number),
    Text(Box<FieldKey>),
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct StyleKey {
    rgb: Option<u32>,
    shadow_color: Option<i32>,
    flags: [Option<bool>; 5],
    click: Option<ClickKey>,
    hover: Option<Box<FieldKey>>,
    insertion: Option<NbtString>,
    font: Option<Identifier>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
enum ClickKey {
    RunCommand(NbtString),
    SuggestCommand(NbtString),
    ChangePage(i32),
    Copy(NbtString),
    Custom(Identifier, Option<ModernPayloadKey>),
}
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
struct ModernPayloadKey(Arc<NbtValue>);
impl PartialEq for ModernPayloadKey {
    fn eq(&self, other: &Self) -> bool {
        super::nbt::equivalent(&self.0, &other.0, crate::MinecraftVersion::Java1_21_11)
    }
}
impl Eq for ModernPayloadKey {}
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Style {
    pub color: Option<Color>,
    pub shadow_color: Option<i32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underlined: Option<bool>,
    pub strikethrough: Option<bool>,
    pub obfuscated: Option<bool>,
    pub click: Option<Click>,
    pub hover: Option<Hover>,
    pub insertion: Option<NbtString>,
    pub font: Option<Identifier>,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Color {
    pub rgb: u32,
    /// Native persistent spelling remains distinct from RGB equality.
    pub serialized: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum Click {
    OpenUrl(NbtString),
    OpenFile(NbtString),
    RunCommand(NbtString),
    SuggestCommand(NbtString),
    ChangePage(i32),
    Copy(NbtString),
    Custom {
        id: Identifier,
        payload: Option<Arc<NbtValue>>,
    },
    Dialog(Arc<NbtValue>),
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum Hover {
    Text(Box<Text>),
    Item(Arc<NbtValue>),
    Entity(Arc<NbtValue>),
}

/// These fields still require native constructor/context work. No blanket Eq is
/// implemented for this model: URI, selectors, profiles, nested items/dialogs and
/// entity bindings cannot be replaced with raw NBT/string/CRC equality.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum Dependency {
    Selector,
    Profile,
    Uri,
    Dialog,
    Item,
    Entity,
    NativeValidation,
}
impl Text {
    pub(crate) fn dependencies(&self) -> Vec<Dependency> {
        fn visit(text: &Text, out: &mut Vec<Dependency>) {
            use Contents::*;
            match &text.contents {
                Selector { separator, .. } => {
                    out.push(Dependency::Selector);
                    if let Some(value) = separator {
                        visit(value, out);
                    }
                }
                Score { .. } => out.push(Dependency::Selector),
                PlayerSprite { .. } => out.push(Dependency::Profile),
                Translate { arguments, .. } => {
                    for value in arguments {
                        if let Argument::Text(value) = value {
                            visit(value, out);
                        }
                    }
                }
                Nbt {
                    separator: Some(value),
                    ..
                } => visit(value, out),
                _ => {}
            }
            match &text.style.click {
                Some(Click::OpenUrl(_)) => out.push(Dependency::Uri),
                Some(Click::Dialog(_)) => out.push(Dependency::Dialog),
                Some(Click::OpenFile(_)) => out.push(Dependency::NativeValidation),
                _ => {}
            }
            match &text.style.hover {
                Some(Hover::Text(value)) => visit(value, out),
                Some(Hover::Item(_)) => out.push(Dependency::Item),
                Some(Hover::Entity(_)) => out.push(Dependency::Entity),
                None => {}
            }
            for value in &text.siblings {
                visit(value, out);
            }
        }
        let mut values = Vec::new();
        visit(self, &mut values);
        values
    }
    /// A key for fully represented, context-free modern text fields. It does
    /// not attest complete original constructor validation, whole item equality,
    /// persistent encoding, cached server hashes or action admission.
    pub(crate) fn modern_field_key(&self) -> Option<FieldKey> {
        if !self.dependencies().is_empty() {
            return None;
        }
        fn key(text: &Text) -> FieldKey {
            let contents = match &text.contents {
                Contents::Literal { text } => ContentsKey::Literal(text.clone()),
                Contents::Keybind { keybind } => ContentsKey::Keybind(keybind.clone()),
                Contents::Translate {
                    key: k,
                    fallback,
                    arguments,
                } => ContentsKey::Translate {
                    key: k.clone(),
                    fallback: fallback.clone(),
                    arguments: arguments
                        .iter()
                        .map(|a| match a {
                            Argument::String(s) => ArgumentKey::String(s.clone()),
                            Argument::Number(n) => ArgumentKey::Number(n.clone()),
                            Argument::Text(t) => ArgumentKey::Text(Box::new(key(t))),
                        })
                        .collect(),
                },
                Contents::Nbt {
                    path,
                    interpret,
                    separator,
                    source,
                } => ContentsKey::Nbt {
                    path: path.clone(),
                    interpret: *interpret,
                    separator: separator.as_ref().map(|v| Box::new(key(v))),
                    source: source.clone(),
                },
                Contents::Sprite { atlas, sprite } => {
                    ContentsKey::Sprite(atlas.clone(), sprite.clone())
                }
                _ => unreachable!("dependency-free text cannot contain unresolved contents"),
            };
            let s = &text.style;
            let click = s.click.as_ref().map(|c| match c {
                Click::RunCommand(v) => ClickKey::RunCommand(v.clone()),
                Click::SuggestCommand(v) => ClickKey::SuggestCommand(v.clone()),
                Click::ChangePage(v) => ClickKey::ChangePage(*v),
                Click::Copy(v) => ClickKey::Copy(v.clone()),
                Click::Custom { id, payload } => {
                    ClickKey::Custom(id.clone(), payload.clone().map(ModernPayloadKey))
                }
                _ => unreachable!("unresolved click dependency"),
            });
            let hover = s.hover.as_ref().map(|h| match h {
                Hover::Text(t) => Box::new(key(t)),
                _ => unreachable!("unresolved hover dependency"),
            });
            FieldKey {
                contents,
                style: StyleKey {
                    rgb: s.color.as_ref().map(|c| c.rgb),
                    shadow_color: s.shadow_color,
                    flags: [
                        s.bold,
                        s.italic,
                        s.underlined,
                        s.strikethrough,
                        s.obfuscated,
                    ],
                    click,
                    hover,
                    insertion: s.insertion.clone(),
                    font: s.font.clone(),
                },
                siblings: text.siblings.iter().map(key).collect(),
            }
        }
        Some(key(self))
    }
}
