//! Original modern URI constructor fields and equality, without URL resolution.
use super::{constructor::Limit, nbt::NbtString};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};
#[derive(Deserialize)]
struct Mask {
    ascii_units: Vec<u16>,
    escaped: bool,
}
#[derive(Deserialize)]
struct Rules {
    masks: BTreeMap<String, Mask>,
    excluded_non_ascii_units: Vec<u16>,
    allowed_schemes: Vec<String>,
}
fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/client_api/uri_rules-1.21.11.json"))
            .expect("pinned original URI grammar")
    })
}
fn valid(units: &[u16], mask: &str) -> bool {
    let mask = &rules().masks[mask];
    let mut cursor = 0;
    while let Some(&unit) = units.get(cursor) {
        if unit == 37 && mask.escaped {
            if !units
                .get(cursor + 1..cursor + 3)
                .is_some_and(|v| v.iter().all(|u| matches!(*u,48..=57|65..=70|97..=102)))
            {
                return false;
            }
            cursor += 3;
        } else {
            if !(mask.ascii_units.binary_search(&unit).is_ok()
                || unit >= 128
                    && mask.escaped
                    && rules()
                        .excluded_non_ascii_units
                        .binary_search(&unit)
                        .is_err())
            {
                return false;
            }
            cursor += 1;
        }
    }
    true
}
fn lower(units: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|&u| if (65..=90).contains(&u) { u + 32 } else { u })
        .collect()
}
fn escaped_key(units: &[u16]) -> Vec<u16> {
    let mut key = units.to_vec();
    let mut cursor = 0;
    while cursor < key.len() {
        if key[cursor] == 37 {
            for u in &mut key[cursor + 1..cursor + 3] {
                if (97..=102).contains(u) {
                    *u -= 32;
                }
            }
            cursor += 3;
        } else {
            cursor += 1;
        }
    }
    key
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum AuthorityKey {
    Absent,
    Registry(Vec<u16>),
    Server(Option<Vec<u16>>, Vec<u16>, i32),
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum BodyKey {
    Opaque(Vec<u16>),
    Hierarchical(AuthorityKey, Vec<u16>, Option<Vec<u16>>),
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Key(Vec<u16>, BodyKey, Option<Vec<u16>>);
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Uri {
    pub raw: NbtString,
    pub opaque: bool,
    pub scheme: NbtString,
    pub scheme_specific_part: NbtString,
    pub authority: Option<NbtString>,
    pub user_info: Option<NbtString>,
    pub host: Option<NbtString>,
    pub port: i32,
    pub path: Option<NbtString>,
    pub query: Option<NbtString>,
    pub fragment: Option<NbtString>,
    #[serde(skip)]
    key: Key,
}
impl PartialEq for Uri {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for Uri {}
fn string(units: &[u16]) -> NbtString {
    NbtString::from_units(units.to_vec())
}
fn ipv4(units: &[u16]) -> bool {
    let parts = units.split(|u| *u == 46).collect::<Vec<_>>();
    parts.len() == 4
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.iter().all(|u| (48..=57).contains(u))
                && part
                    .iter()
                    .try_fold(0u32, |v, u| {
                        v.checked_mul(10)?.checked_add(u32::from(*u - 48))
                    })
                    .is_some_and(|v| v <= 255)
        })
}
fn hostname(units: &[u16]) -> bool {
    if ipv4(units) {
        return true;
    }
    let units = units.strip_suffix(&[46]).unwrap_or(units);
    let parts = units.split(|u| *u == 46).collect::<Vec<_>>();
    let alphanumeric = |u: &u16| matches!(*u,48..=57|65..=90|97..=122);
    parts.iter().all(|p| {
        p.first().is_some_and(alphanumeric)
            && p.last().is_some_and(alphanumeric)
            && p.iter().all(|u| alphanumeric(u) || *u == 45)
    }) && (parts.len() == 1
        || parts
            .last()
            .unwrap()
            .first()
            .is_some_and(|u| matches!(*u,65..=90|97..=122)))
}
fn ipv6(units: &[u16]) -> bool {
    let (address, scope) = match units.iter().position(|u| *u == 37) {
        Some(i) => (&units[..i], Some(&units[i + 1..])),
        None => (units, None),
    };
    if scope.is_some_and(|v| v.is_empty() || !valid(v, "SCOPE_ID")) {
        return false;
    }
    let Some(compression) = address.windows(2).position(|v| v == [58, 58]) else {
        return ipv6_parts(address).is_some_and(|n| n == 8);
    };
    let left = &address[..compression];
    let right = &address[compression + 2..];
    if right.windows(2).any(|v| v == [58, 58]) || left.contains(&46) {
        return false;
    }
    ipv6_parts(left)
        .zip(ipv6_parts(right))
        .is_some_and(|(a, b)| a + b < 8)
}
fn ipv6_parts(units: &[u16]) -> Option<usize> {
    if units.is_empty() {
        return Some(0);
    }
    let parts = units.split(|u| *u == 58).collect::<Vec<_>>();
    let mut count = 0;
    for (i, p) in parts.iter().enumerate() {
        if p.contains(&46) {
            if i + 1 != parts.len() || !ipv4(p) {
                return None;
            }
            count += 2;
        } else {
            if p.is_empty()
                || p.len() > 4
                || !p.iter().all(|u| matches!(*u,48..=57|65..=70|97..=102))
            {
                return None;
            }
            count += 1;
        }
    }
    Some(count)
}
type Server<'a> = (Option<&'a [u16]>, &'a [u16], i32);
fn server(units: &[u16]) -> Result<Server<'_>> {
    let (user, host_port) = match units.iter().position(|u| *u == 64) {
        Some(i) => {
            if !valid(&units[..i], "USERINFO") {
                bail!("invalid URI user info");
            }
            (Some(&units[..i]), &units[i + 1..])
        }
        None => (None, units),
    };
    let (host, port) = if host_port.first() == Some(&91) {
        let end = host_port
            .iter()
            .position(|u| *u == 93)
            .context("unterminated IPv6 host")?;
        if !ipv6(&host_port[1..end]) {
            bail!("invalid original IPv6 host");
        }
        (&host_port[..=end], &host_port[end + 1..])
    } else {
        let end = host_port
            .iter()
            .position(|u| *u == 58)
            .unwrap_or(host_port.len());
        if !hostname(&host_port[..end]) {
            bail!("invalid original hostname");
        }
        (&host_port[..end], &host_port[end..])
    };
    let port = if port.is_empty() {
        -1
    } else {
        let digits = port
            .strip_prefix(&[58])
            .context("invalid URI port delimiter")?;
        if digits.is_empty() {
            -1
        } else {
            if !digits.iter().all(|u| (48..=57).contains(u)) {
                bail!("invalid URI port");
            }
            digits
                .iter()
                .try_fold(0i32, |v, u| {
                    v.checked_mul(10)?.checked_add(i32::from(*u - 48))
                })
                .context("URI port overflow")?
        }
    };
    Ok((user, host, port))
}
pub(crate) fn parse(value: &NbtString, budget: &mut usize) -> Result<Uri> {
    let units = value.utf16();
    *budget = budget
        .checked_sub(units.len())
        .ok_or(Limit("URI constructor work limit"))?;
    let colon = units
        .iter()
        .position(|u| *u == 58)
        .context("URI scheme required")?;
    let scheme = &units[..colon];
    if !scheme
        .first()
        .is_some_and(|u| matches!(*u,65..=90|97..=122))
        || !valid(scheme, "SCHEME")
    {
        bail!("invalid URI scheme");
    }
    let scheme_lower = String::from_utf16(&lower(scheme))?;
    if !rules().allowed_schemes.contains(&scheme_lower) {
        bail!("native URI scheme forbidden");
    }
    let rest = &units[colon + 1..];
    let (body, fragment) = match rest.iter().position(|u| *u == 35) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    if fragment.is_some_and(|v| !valid(v, "URIC")) {
        bail!("invalid URI fragment");
    }
    if body.is_empty() {
        bail!("empty URI scheme-specific part");
    }
    let opaque = body.first() != Some(&47);
    let mut authority = None;
    let mut user_info = None;
    let mut host = None;
    let mut port = -1;
    let mut path = None;
    let mut query = None;
    let key = if opaque {
        if !valid(body, "URIC") {
            bail!("invalid opaque URI");
        }
        BodyKey::Opaque(escaped_key(body))
    } else {
        let (hierarchy, q) = match body.iter().position(|u| *u == 63) {
            Some(i) => (&body[..i], Some(&body[i + 1..])),
            None => (body, None),
        };
        if q.is_some_and(|v| !valid(v, "URIC")) {
            bail!("invalid URI query");
        }
        query = q.map(string);
        let (a, p) = if let Some(rest) = hierarchy.strip_prefix(&[47, 47]) {
            let end = rest.iter().position(|u| *u == 47).unwrap_or(rest.len());
            if end == 0 && rest.is_empty() && q.is_none() && fragment.is_none() {
                bail!("empty URI authority");
            }
            (
                if end == 0 { None } else { Some(&rest[..end]) },
                &rest[end..],
            )
        } else {
            (None, hierarchy)
        };
        if !valid(p, "PATH") {
            bail!("invalid URI path");
        }
        path = Some(string(p));
        let authority_key = if let Some(a) = a {
            authority = Some(string(a));
            match server(a) {
                Ok((u, h, n)) => {
                    user_info = u.map(string);
                    host = Some(string(h));
                    port = n;
                    AuthorityKey::Server(u.map(escaped_key), lower(h), n)
                }
                Err(_) if valid(a, "REG_NAME") => AuthorityKey::Registry(escaped_key(a)),
                Err(error) => return Err(error),
            }
        } else {
            AuthorityKey::Absent
        };
        BodyKey::Hierarchical(authority_key, escaped_key(p), q.map(escaped_key))
    };
    Ok(Uri {
        raw: value.clone(),
        opaque,
        scheme: string(scheme),
        scheme_specific_part: string(body),
        authority,
        user_info,
        host,
        port,
        path,
        query,
        fragment: fragment.map(string),
        key: Key(lower(scheme), key, fragment.map(escaped_key)),
    })
}
