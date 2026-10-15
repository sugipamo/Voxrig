//! Staged private key schema. Production decisions still use contact_capabilities.
//!
//! The dense index includes absent version/entity pairs; presence is a separate
//! axis constraint. Baseline rows for those pairs are explicitly `absent`.
//! `Effect` names the seven existing capability fields, not the four evidence
//! classifications. Owner lanes remain separate from effects.
//!
//! Review snapshot changes before updating deliberately with:
//! `VOXRIG_UPDATE_CONTACT_BASELINE=1 cargo test --locked -j1 frozen_contact_policy_baseline`.
//! The test enumerates the full present product; run-length encoding only
//! compresses identical consecutive packet outcomes in the checked-in text.
// Staged schema: intentionally unused by production until a later migration.
#![allow(dead_code)]
pub(super) mod key;

#[cfg(test)]
mod baseline;

mod table;
mod validate;
