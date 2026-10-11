//! Bounded spatial discovery at an explicitly supplied model step.
use super::{
    LivingContactBody, LivingContactCandidate, LivingContactDisposition, LivingContactPlayer,
    LivingContactsUnavailable, MAX_LIVING_CONTACT_CANDIDATES, contact,
};
use crate::{MinecraftVersion, client::Aabb};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum bodies and nonempty sections in one independent model index.
pub const MAX_LIVING_CONTACT_INDEX_BODIES: usize = 256;
/// Maximum explicitly known chunks, including empty chunks, in one index.
pub const MAX_LIVING_CONTACT_INDEX_CHUNKS: usize = 256;
/// Maximum chunks examined by one bounded discovery query.
pub const MAX_LIVING_CONTACT_QUERY_CHUNKS: usize = 64;

/// Qualified inherited living-contact species, separate from received types.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LivingContactKind {
    /// Player with an explicit original local/remote classification.
    Player,
    /// Cow's inherited living-contact dispatch.
    Cow,
    /// Pig's inherited living-contact dispatch.
    Pig,
    /// Zombie's inherited living-contact dispatch.
    Zombie,
}

/// Caller-admitted body at one model time. Keys identify numerical bodies,
/// teams and vehicle roots only; they cannot construct a live EntityId.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LivingContactIndexedBody {
    /// Unique model-lifetime key. A new spawn must use a new key.
    pub key: u64,
    /// None retains a known box whose species/dispatch is unqualified.
    pub kind: Option<LivingContactKind>,
    /// Explicit model position, bounds, velocity and original predicates.
    pub body: LivingContactBody,
    /// Original candidate spectator predicate.
    pub spectator: bool,
    /// Original candidate local-player classification.
    pub player: LivingContactPlayer,
    /// Explicit original team identity, absent exactly when body.team is None.
    pub team_key: Option<u64>,
    /// Explicit root-vehicle key; None means this body is its own root.
    pub vehicle_root: Option<u64>,
}

/// Discovery refusal; no partial candidates or index mutation are returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum LivingContactQueryUnavailable {
    /// Index was explicitly invalidated on a lifetime or history boundary.
    Invalidated,
    /// Position lies outside the qualified packed-section coordinate range.
    UnsupportedCoordinates,
    /// Invalid body, bounds, team or legacy-only input.
    InvalidBody(LivingContactsUnavailable),
    /// Species and local-player identity, or team identity/rule, disagree.
    InconsistentIdentity,
    /// Body retention budget is exhausted; old bodies are not evicted.
    TooManyBodies,
    /// Explicit known-chunk budget is exhausted.
    TooManyChunks,
    /// Query would examine more than the fixed chunk budget.
    QueryTooLarge,
    /// Query touches a chunk without explicitly complete model membership.
    UnknownChunk,
    /// Visibility update refers to an absent section.
    UnknownSection,
    /// An intersecting body has unqualified species/dispatch.
    UnsupportedBody,
    /// Admitted contacts exceed the inherited-contact batch budget.
    TooManyContacts,
}

/// Selected contacts in original version's spatial query order.
#[derive(Clone, Debug, serde::Serialize)]
pub struct LivingContactQuery {
    /// Original query-rule version; not a received server observation.
    pub version: MinecraftVersion,
    /// Model keys parallel to candidates, never live targets.
    pub keys: Vec<u64>,
    /// Automatically selected candidates, ready for the explicit push model.
    pub candidates: Vec<LivingContactCandidate>,
}

#[derive(Clone, Debug)]
struct Section {
    keys: Vec<u64>,
    accessible: bool,
}

/// Independent model index. Supplied updates determine membership time;
/// this stage discovers contacts but does not schedule native ticks or send.
/// The caller must invalidate on world/spawn/history gaps and rebuild from
/// complete admitted model inputs. Received packet targets are not silently
/// used as current boxes. Legacy uses its original 16 clamped Y sections.
#[derive(Clone, Debug)]
pub struct LivingContactIndex {
    version: MinecraftVersion,
    valid: bool,
    bodies: BTreeMap<u64, LivingContactIndexedBody>,
    sections: BTreeMap<[i32; 3], Section>,
    chunks: BTreeSet<[i32; 2]>,
}

impl LivingContactIndex {
    /// Create a separate empty model. No chunk membership is assumed complete.
    pub fn new(version: MinecraftVersion) -> Self {
        Self {
            version,
            valid: true,
            bodies: BTreeMap::new(),
            sections: BTreeMap::new(),
            chunks: BTreeSet::new(),
        }
    }

    /// Irreversibly discard this model on a lifetime/history boundary.
    /// Recovery requires a new index and explicit model admission.
    pub fn invalidate(&mut self) {
        self.valid = false;
        self.bodies.clear();
        self.sections.clear();
        self.chunks.clear();
    }

    fn check_valid(&self) -> Result<(), LivingContactQueryUnavailable> {
        if self.valid {
            Ok(())
        } else {
            Err(LivingContactQueryUnavailable::Invalidated)
        }
    }

    /// Mark a chunk's model membership complete, including known emptiness.
    /// false marks a gap, not proof of an empty or inaccessible native chunk.
    pub fn set_chunk_known(
        &mut self,
        chunk: [i32; 2],
        known: bool,
    ) -> Result<(), LivingContactQueryUnavailable> {
        self.check_valid()?;
        if chunk.iter().any(|v| !(-2_000_000..=2_000_000).contains(v)) {
            return Err(LivingContactQueryUnavailable::UnsupportedCoordinates);
        }
        if known {
            if !self.chunks.contains(&chunk) && self.chunks.len() == MAX_LIVING_CONTACT_INDEX_CHUNKS
            {
                return Err(LivingContactQueryUnavailable::TooManyChunks);
            }
            self.chunks.insert(chunk);
        } else {
            self.chunks.remove(&chunk);
        }
        Ok(())
    }

    /// Add or update a body at the supplied model step. Staying in a section
    /// preserves list order; moving sections removes then appends. An explicit
    /// remove followed by a new admission appends even at the same position.
    /// New modern sections are accessible until explicitly hidden.
    pub fn upsert(
        &mut self,
        entity: LivingContactIndexedBody,
    ) -> Result<(), LivingContactQueryUnavailable> {
        self.check_valid()?;
        validate(self.version, &entity)?;
        let new_section = section(self.version, entity.body.push.position)?;
        let old_section = self
            .bodies
            .get(&entity.key)
            .map(|e| section(self.version, e.body.push.position))
            .transpose()?;
        if old_section.is_none() && self.bodies.len() == MAX_LIVING_CONTACT_INDEX_BODIES {
            return Err(LivingContactQueryUnavailable::TooManyBodies);
        }
        if old_section != Some(new_section) {
            if let Some(old) = old_section {
                self.remove_from_section(old, entity.key);
            }
            self.sections
                .entry(new_section)
                .or_insert_with(|| Section {
                    keys: Vec::new(),
                    accessible: true,
                })
                .keys
                .push(entity.key);
        }
        self.bodies.insert(entity.key, entity);
        Ok(())
    }

    fn remove_from_section(&mut self, section: [i32; 3], key: u64) {
        if let Some(s) = self.sections.get_mut(&section) {
            s.keys.retain(|&k| k != key);
            if s.keys.is_empty() {
                self.sections.remove(&section);
            }
        }
    }

    /// Retire a supplied model lifetime. An absent key changes nothing.
    pub fn remove(&mut self, key: u64) -> Result<bool, LivingContactQueryUnavailable> {
        self.check_valid()?;
        let Some(e) = self.bodies.remove(&key) else {
            return Ok(false);
        };
        self.remove_from_section(section(self.version, e.body.push.position)?, key);
        Ok(true)
    }

    /// Explicit modern native section visibility at the supplied model step.
    /// Legacy sections have no equivalent visibility branch and refuse this.
    pub fn set_section_accessible(
        &mut self,
        section: [i32; 3],
        accessible: bool,
    ) -> Result<(), LivingContactQueryUnavailable> {
        self.check_valid()?;
        if self.version != MinecraftVersion::Java1_21_11 {
            return Err(LivingContactQueryUnavailable::UnknownSection);
        }
        let s = self
            .sections
            .get_mut(&section)
            .ok_or(LivingContactQueryUnavailable::UnknownSection)?;
        s.accessible = accessible;
        Ok(())
    }

    /// Discover contacts from model-time boxes, identity and team predicates.
    /// Query bounds use the original legacy +/-2 grace or modern minY-4/maxY+0.
    /// All needed chunks must be explicitly known. Unsupported intersecting
    /// species and excess contacts refuse the whole query, without truncation.
    pub fn query(
        &self,
        client_side: bool,
        querying: &LivingContactIndexedBody,
    ) -> Result<LivingContactQuery, LivingContactQueryUnavailable> {
        self.check_valid()?;
        validate(self.version, querying)?;
        if querying.kind.is_none() {
            return Err(LivingContactQueryUnavailable::UnsupportedBody);
        }
        let b = querying.body.bounds;
        let min = section(
            self.version,
            [
                b.min_x - 2.,
                b.min_y
                    - if self.version == MinecraftVersion::Java1_16_1 {
                        2.
                    } else {
                        4.
                    },
                b.min_z - 2.,
            ],
        )?;
        let max = section(
            self.version,
            [
                b.max_x + 2.,
                b.max_y
                    + if self.version == MinecraftVersion::Java1_16_1 {
                        2.
                    } else {
                        0.
                    },
                b.max_z + 2.,
            ],
        )?;
        let count = i64::from(max[0] - min[0] + 1) * i64::from(max[2] - min[2] + 1);
        if count > MAX_LIVING_CONTACT_QUERY_CHUNKS as i64 {
            return Err(LivingContactQueryUnavailable::QueryTooLarge);
        }
        for x in min[0]..=max[0] {
            for z in min[2]..=max[2] {
                if !self.chunks.contains(&[x, z]) {
                    return Err(LivingContactQueryUnavailable::UnknownChunk);
                }
            }
        }
        let mut sections: Vec<_> = self
            .sections
            .iter()
            .filter(|(p, s)| {
                (min[0]..=max[0]).contains(&p[0])
                    && (min[1]..=max[1]).contains(&p[1])
                    && (min[2]..=max[2]).contains(&p[2])
                    && s.accessible
            })
            .collect();
        match self.version {
            MinecraftVersion::Java1_16_1 => sections.sort_by_key(|(p, _)| [p[0], p[2], p[1]]),
            MinecraftVersion::Java1_21_11 => sections.sort_by_key(|(p, _)| packed_section(**p)),
        }
        let mut result = LivingContactQuery {
            version: self.version,
            keys: Vec::new(),
            candidates: Vec::new(),
        };
        for (_, s) in sections {
            for key in &s.keys {
                let e = &self.bodies[key];
                if *key == querying.key || !intersects(b, e.body.bounds) {
                    continue;
                }
                if e.kind.is_none() {
                    return Err(LivingContactQueryUnavailable::UnsupportedBody);
                }
                if querying.team_key.is_some()
                    && querying.team_key == e.team_key
                    && querying.body.team != e.body.team
                {
                    return Err(LivingContactQueryUnavailable::InconsistentIdentity);
                }
                let candidate = LivingContactCandidate {
                    body: e.body.clone(),
                    same_body: false,
                    spectator: e.spectator,
                    player: e.player,
                    allied: querying.team_key.is_some() && querying.team_key == e.team_key,
                    same_vehicle: querying.vehicle_root.unwrap_or(querying.key)
                        == e.vehicle_root.unwrap_or(e.key),
                };
                if contact::disposition(client_side, &querying.body, &candidate)
                    == LivingContactDisposition::Admitted
                {
                    if result.keys.len() == MAX_LIVING_CONTACT_CANDIDATES {
                        return Err(LivingContactQueryUnavailable::TooManyContacts);
                    }
                    result.keys.push(*key);
                    result.candidates.push(candidate);
                }
            }
        }
        Ok(result)
    }
}

fn validate(
    version: MinecraftVersion,
    e: &LivingContactIndexedBody,
) -> Result<(), LivingContactQueryUnavailable> {
    contact::validate_body(version, &e.body).map_err(LivingContactQueryUnavailable::InvalidBody)?;
    if e.body.team.is_some() != e.team_key.is_some()
        || e.kind.is_some_and(|k| {
            (k == LivingContactKind::Player) != (e.player != LivingContactPlayer::NonPlayer)
        })
    {
        return Err(LivingContactQueryUnavailable::InconsistentIdentity);
    }
    section(version, e.body.push.position)?;
    section(
        version,
        [
            e.body.bounds.min_x,
            e.body.bounds.min_y,
            e.body.bounds.min_z,
        ],
    )?;
    section(
        version,
        [
            e.body.bounds.max_x,
            e.body.bounds.max_y,
            e.body.bounds.max_z,
        ],
    )?;
    Ok(())
}

fn section(
    version: MinecraftVersion,
    p: [f64; 3],
) -> Result<[i32; 3], LivingContactQueryUnavailable> {
    // The restricted range excludes signed packing alias/wrap and Java floor
    // overflow. Bounds and all model positions are checked before conversion.
    if !p.iter().all(|v| v.is_finite())
        || p[0].abs() > 32_000_000.
        || p[2].abs() > 32_000_000.
        || p[1].abs() > 8_000_000.
    {
        return Err(LivingContactQueryUnavailable::UnsupportedCoordinates);
    }
    let mut s = p.map(|v| match version {
        MinecraftVersion::Java1_16_1 => (v / 16.).floor() as i32,
        MinecraftVersion::Java1_21_11 => (v.floor() as i32).div_euclid(16),
    });
    if version == MinecraftVersion::Java1_16_1 {
        s[1] = s[1].clamp(0, 15);
    }
    Ok(s)
}

fn packed_section(p: [i32; 3]) -> i64 {
    ((i64::from(p[0]) & 0x3fffff) << 42)
        | ((i64::from(p[2]) & 0x3fffff) << 20)
        | (i64::from(p[1]) & 0xfffff)
}

fn intersects(a: Aabb, b: Aabb) -> bool {
    a.min_x < b.max_x
        && a.max_x > b.min_x
        && a.min_y < b.max_y
        && a.max_y > b.min_y
        && a.min_z < b.max_z
        && a.max_z > b.min_z
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{LivingPushBody, model_living_contacts};
    use sha2::{Digest, Sha256};
    use std::io::Read;

    fn body(key: u64, p: [f64; 3]) -> LivingContactIndexedBody {
        LivingContactIndexedBody {
            key,
            kind: Some(LivingContactKind::Cow),
            body: LivingContactBody {
                push: LivingPushBody {
                    position: p,
                    velocity: [0.; 3],
                    no_physics: false,
                    vehicle: false,
                    pushable: true,
                },
                bounds: Aabb {
                    min_x: p[0] - 0.25,
                    min_y: p[1],
                    min_z: p[2] - 0.25,
                    max_x: p[0] + 0.25,
                    max_y: p[1] + 1.,
                    max_z: p[2] + 0.25,
                },
                sleeping: false,
                legacy_pushthrough: 0.,
                team: None,
            },
            spectator: false,
            player: LivingContactPlayer::NonPlayer,
            team_key: None,
            vehicle_root: None,
        }
    }
    fn known(version: MinecraftVersion) -> LivingContactIndex {
        let mut index = LivingContactIndex::new(version);
        for x in -3..=3 {
            for z in -3..=3 {
                index.set_chunk_known([x, z], true).unwrap();
            }
        }
        index
    }
    fn queries(index: &LivingContactIndex, q: &LivingContactIndexedBody) -> Vec<u64> {
        index.query(false, q).unwrap().keys
    }
    #[test]
    fn original_version_orders_and_membership_moves_are_distinct() {
        for (version, expected) in [
            (MinecraftVersion::Java1_16_1, vec![40, 30, 20, 10]),
            (MinecraftVersion::Java1_21_11, vec![20, 10, 40, 30]),
        ] {
            let mut index = known(version);
            for e in [
                body(30, [-0.5, 16., -0.5]),
                body(10, [-0.5, 16., 0.5]),
                body(40, [-0.5, 15.5, -0.5]),
                body(20, [-0.5, 15.5, 0.5]),
            ] {
                index.upsert(e).unwrap();
            }
            let mut q = body(999, [-0.5, 15.5, 0.]);
            q.body.bounds = Aabb {
                min_x: -1.,
                min_y: 15.,
                min_z: -1.,
                max_x: 1.,
                max_y: 17.,
                max_z: 1.,
            };
            assert_eq!(queries(&index, &q), expected);
            let mut second = body(50, [-0.5, 15.75, -0.5]);
            index.upsert(second.clone()).unwrap();
            second.body.push.position[0] = -0.625;
            index.upsert(second.clone()).unwrap();
            let keys = queries(&index, &q);
            assert!(keys.iter().position(|k| *k == 40) < keys.iter().position(|k| *k == 50));
            index.remove(40).unwrap();
            index.upsert(body(40, [-0.5, 15.5, -0.5])).unwrap();
            let keys = queries(&index, &q);
            assert!(keys.iter().position(|k| *k == 50) < keys.iter().position(|k| *k == 40));
            second.body.push.position[1] = 16.;
            second.body.bounds.min_y = 16.;
            second.body.bounds.max_y = 17.;
            index.upsert(second).unwrap();
            let keys = queries(&index, &q);
            assert!(keys.iter().position(|k| *k == 30) < keys.iter().position(|k| *k == 50));
        }
    }
    #[test]
    fn gaps_unsupported_inputs_and_failed_updates_return_no_partial_query() {
        use LivingContactQueryUnavailable as U;
        let v = MinecraftVersion::Java1_21_11;
        let mut index = known(v);
        let q = body(999, [0., 65., 0.]);
        let e = body(7, [0.1, 65., 0.1]);
        index.upsert(e.clone()).unwrap();
        let saved = queries(&index, &q);
        let mut bad = e.clone();
        bad.body.bounds.min_x = 3.;
        assert!(matches!(
            index.upsert(bad),
            Err(U::InvalidBody(LivingContactsUnavailable::InvalidBounds))
        ));
        assert_eq!(queries(&index, &q), saved);
        let mut bad = e.clone();
        bad.body.push.position[0] = f64::NAN;
        assert!(index.upsert(bad).is_err());
        assert_eq!(queries(&index, &q), saved);
        let mut bad = e.clone();
        bad.team_key = Some(1);
        assert_eq!(index.upsert(bad), Err(U::InconsistentIdentity));
        index.set_chunk_known([0, 0], false).unwrap();
        assert!(matches!(index.query(false, &q), Err(U::UnknownChunk)));
        index.set_chunk_known([0, 0], true).unwrap();
        assert_eq!(queries(&index, &q), saved);
        let mut unknown = body(8, [0.2, 65., 0.2]);
        unknown.kind = None;
        index.upsert(unknown).unwrap();
        assert!(matches!(index.query(false, &q), Err(U::UnsupportedBody)));
        index.remove(8).unwrap();
        index.invalidate();
        assert!(matches!(index.query(false, &q), Err(U::Invalidated)));
        assert_eq!(index.upsert(e), Err(U::Invalidated));
        assert_eq!(index.set_chunk_known([0, 0], true), Err(U::Invalidated));
    }
    #[test]
    fn visibility_team_identity_and_contact_limits_are_explicit() {
        use crate::client::ui::TeamCollision;
        use LivingContactQueryUnavailable as U;
        let mut index = known(MinecraftVersion::Java1_21_11);
        let mut q = body(999, [0., 65., 0.]);
        let mut e = body(7, [0.1, 65., 0.1]);
        e.body.team = Some(TeamCollision::Always);
        e.team_key = Some(1);
        index.upsert(e).unwrap();
        q.body.team = Some(TeamCollision::Never);
        q.team_key = Some(1);
        assert!(matches!(
            index.query(false, &q),
            Err(U::InconsistentIdentity)
        ));
        q.body.team = None;
        q.team_key = None;
        assert_eq!(queries(&index, &q), vec![7]);
        index.set_section_accessible([0, 4, 0], false).unwrap();
        assert!(queries(&index, &q).is_empty());
        index.set_section_accessible([0, 4, 0], true).unwrap();
        for key in 8..=39 {
            index.upsert(body(key, [0.1, 65., 0.1])).unwrap();
        }
        assert!(matches!(index.query(false, &q), Err(U::TooManyContacts)));
        index.remove(39).unwrap();
        assert_eq!(queries(&index, &q).len(), 32);
        let selected = index.query(false, &q).unwrap();
        let composed =
            model_living_contacts(index.version, false, &q.body, &selected.candidates).unwrap();
        assert!(
            composed
                .candidates
                .iter()
                .all(|c| c.disposition == LivingContactDisposition::Admitted)
        );
        assert!(index.query(true, &q).unwrap().keys.is_empty());
    }
    #[test]
    fn bounds_memory_and_region_budgets_do_not_evict_supported_state() {
        use LivingContactQueryUnavailable as U;
        let mut index = known(MinecraftVersion::Java1_16_1);
        let q = body(999, [0., 65., 0.]);
        for key in 0..MAX_LIVING_CONTACT_INDEX_BODIES as u64 {
            index.upsert(body(key, [48., 65., 48.])).unwrap();
        }
        assert_eq!(
            index.upsert(body(1000, [0.1, 65., 0.1])),
            Err(U::TooManyBodies)
        );
        assert!(queries(&index, &q).is_empty());
        assert_eq!(index.remove(5000), Ok(false));
        index.remove(0).unwrap();
        index.upsert(body(1000, [0.1, 65., 0.1])).unwrap();
        assert_eq!(queries(&index, &q), vec![1000]);
        let mut q = q;
        q.body.bounds.max_x = 2048.;
        assert!(matches!(index.query(false, &q), Err(U::QueryTooLarge)));
        q.body.push.position[0] = 33_000_000.;
        assert!(matches!(
            index.query(false, &q),
            Err(U::UnsupportedCoordinates)
        ));
        let mut fresh = LivingContactIndex::new(MinecraftVersion::Java1_21_11);
        for x in 0..MAX_LIVING_CONTACT_INDEX_CHUNKS as i32 {
            fresh.set_chunk_known([x, 0], true).unwrap();
        }
        assert_eq!(fresh.set_chunk_known([999, 0], true), Err(U::TooManyChunks));
        fresh.set_chunk_known([0, 0], false).unwrap();
        fresh.set_chunk_known([999, 0], true).unwrap();
    }

    fn decode(v: &serde_json::Value) -> LivingContactIndexedBody {
        let mut e: LivingContactIndexedBody = serde_json::from_value(v.clone()).unwrap();
        let b = &v["body"];
        let vector = |v: &serde_json::Value| {
            std::array::from_fn(|i| {
                f64::from_bits(u64::from_str_radix(v[i].as_str().unwrap(), 16).unwrap())
            })
        };
        e.body.push.position = vector(&b["push"]["position_bits"]);
        e.body.push.velocity = vector(&b["push"]["velocity_bits"]);
        let f = |i| {
            f64::from_bits(u64::from_str_radix(b["bounds_bits"][i].as_str().unwrap(), 16).unwrap())
        };
        e.body.bounds = Aabb {
            min_x: f(0),
            min_y: f(1),
            min_z: f(2),
            max_x: f(3),
            max_y: f(4),
            max_z: f(5),
        };
        e
    }
    #[test]
    fn discovery_matches_unchanged_original_queries_through_model_updates() {
        let mut json = String::new();
        flate2::read::GzDecoder::new(
            &include_bytes!("../../../data/client_api/entity_query_oracle.json.gz")[..],
        )
        .read_to_string(&mut json)
        .unwrap();
        let oracle: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            oracle["runner_sha256"],
            format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../scripts/run_entity_query_oracle.py"
                ))
            )
        );
        let mut total = 0;
        for (version, key, harness, jar, mapping) in [
            (
                MinecraftVersion::Java1_16_1,
                "1.16.1",
                &include_bytes!(
                    "../../../scripts/entity_query_oracle/1.16.1/EntityQueryOracle.java"
                )[..],
                "a412fd69db1f81db3f511c1463fd304675244077",
                "11120c39da4df293c4bd020896391fb9ddd6c2ba",
            ),
            (
                MinecraftVersion::Java1_21_11,
                "1.21.11",
                &include_bytes!(
                    "../../../scripts/entity_query_oracle/1.21.11/EntityQueryOracle.java"
                )[..],
                "64bb6d763bed0a9f1d632ec347938594144943ed",
                "5621e9253f05fd57872bbe7f8ddf5f9a7d525955",
            ),
        ] {
            let original = &oracle["versions"][key];
            assert_eq!(original["original_server_jar_sha1"], jar);
            assert_eq!(original["mappings_sha1"], mapping);
            assert_eq!(
                original["harness_sha256"],
                format!("{:x}", Sha256::digest(harness))
            );
            let rows = original["results"].as_array().unwrap();
            assert_eq!(rows.len(), 48);
            for row in rows {
                let mut index = LivingContactIndex::new(version);
                let q = decode(&row["querying"]);
                for chunk in row["known_chunks"].as_array().unwrap() {
                    index
                        .set_chunk_known(serde_json::from_value(chunk.clone()).unwrap(), true)
                        .unwrap();
                }
                for step in row["steps"].as_array().unwrap() {
                    for action in step["actions"].as_array().unwrap() {
                        match action["op"].as_str().unwrap() {
                            "upsert" => index.upsert(decode(&action["entity"])).unwrap(),
                            "remove" => {
                                assert!(index.remove(action["key"].as_u64().unwrap()).unwrap());
                            }
                            "visibility" => index
                                .set_section_accessible(
                                    serde_json::from_value(action["section"].clone()).unwrap(),
                                    action["accessible"].as_bool().unwrap(),
                                )
                                .unwrap(),
                            other => panic!("unexpected original fixture action {other}"),
                        }
                    }
                    let result = index
                        .query(row["client_side"].as_bool().unwrap(), &q)
                        .unwrap();
                    let expected: Vec<u64> = serde_json::from_value(step["keys"].clone()).unwrap();
                    assert_eq!(
                        result.keys, expected,
                        "{key} seed {} step {total}",
                        row["seed"]
                    );
                    let contacts = model_living_contacts(
                        version,
                        row["client_side"].as_bool().unwrap(),
                        &q.body,
                        &result.candidates,
                    )
                    .unwrap();
                    assert!(
                        contacts
                            .candidates
                            .iter()
                            .all(|c| c.disposition == LivingContactDisposition::Admitted)
                    );
                    total += 1;
                }
            }
        }
        assert_eq!(total, 576);
    }
}
