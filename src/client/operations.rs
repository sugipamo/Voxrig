//! Mode-specific common operations. Handles do not grant or change game mode.
use super::{GameMode, PlayerObservation};
use crate::{BlockFace, Client, MinecraftVersion, Result};

/// Complete packet dispatch, separate from protocol acknowledgement or game outcome.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DispatchReceipt {
    /// Owning version.
    pub version: MinecraftVersion,
    /// Owning transport, meaningful within this process only.
    pub connection_id: u64,
    /// Interaction sequence if this native protocol provides one.
    pub interaction_sequence: Option<i32>,
}
/// Operations whose permissions are checked against received survival mode.
///
/// Survival handles cannot manufacture creative items:
/// ```compile_fail
/// async fn bypass(ops: &voxrig::client::Survival) {
///     ops.set_hotbar(0, Some(("minecraft:stone", 1))).await.unwrap();
/// }
/// ```

#[derive(Clone)]
pub struct Survival {
    pub(crate) client: Client,
}
/// Operations whose permissions are checked against received creative mode.
#[derive(Clone)]
pub struct Creative {
    pub(crate) client: Client,
}

pub(crate) enum Action<'a> {
    Look([f32; 2]),
    SelectHotbar(u8),
    SetFlying(bool),
    MoveFlying([f64; 3], [f32; 2]),
    SetHotbar(u8, Option<(&'a str, u8)>),
    Dig([i32; 3], BlockFace),
    UseOnBlock([i32; 3], BlockFace, [f32; 3]),
}
impl Survival {
    /// Capture player/inventory without inventing mode, item or position facts.
    pub async fn player_state(&self) -> Result<PlayerObservation> {
        self.client.player_state().await
    }
    /// Dispatch a look. Native admission applies; success is not acceptance.
    pub async fn look(&self, rotation: [f32; 2]) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::Look(rotation))
            .await
    }
    /// Select main-hand hotbar index 0..8 without inventing an item echo.
    pub async fn select_hotbar(&self, slot: u8) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Survival, Action::SelectHotbar(slot))
            .await
    }
    /// Select the additional audited dry-cube contract, when implemented.
    /// This remains an explicitly restricted extension, not general survival parity.
    pub fn checked(&self) -> Result<crate::checked_survival::Operations> {
        self.client.checked_survival()
    }
}
impl Creative {
    /// Capture player/inventory. Holding this handle does not imply creative permission.
    pub async fn player_state(&self) -> Result<PlayerObservation> {
        self.client.player_state().await
    }
    /// Dispatch a view rotation under received creative-mode admission.
    pub async fn look(&self, rotation: [f32; 2]) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::Look(rotation))
            .await
    }
    /// Select main-hand hotbar index 0..8.
    pub async fn select_hotbar(&self, slot: u8) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SelectHotbar(slot))
            .await
    }
    /// Request flight; enabling requires a received server flight permission.
    pub async fn set_flying(&self, flying: bool) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SetFlying(flying))
            .await
    }
    /// Submit a flight step of at most four blocks. Not collision resolution or teleport.
    pub async fn move_flying(
        &self,
        position: [f64; 3],
        rotation: [f32; 2],
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::MoveFlying(position, rotation))
            .await
    }
    /// Write a default stack by namespaced name, or clear a hotbar slot.
    /// Inventory remains unknown until received; no local result prediction.
    pub async fn set_hotbar(&self, slot: u8, item: Option<(&str, u8)>) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::SetHotbar(slot, item))
            .await
    }
    /// Dispatch one creative START break to a loaded reachable target.
    pub async fn break_block(&self, target: [i32; 3], face: BlockFace) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::Dig(target, face))
            .await
    }
    /// Use held item on a loaded reachable block face. Can place or activate.
    /// Cursor coordinates are relative to the target block, each within 0..1.
    pub async fn use_on_block(
        &self,
        target: [i32; 3],
        face: BlockFace,
        cursor: [f32; 3],
    ) -> Result<DispatchReceipt> {
        self.client
            .execute(GameMode::Creative, Action::UseOnBlock(target, face, cursor))
            .await
    }
}
pub(crate) fn validate_rotation(rotation: [f32; 2]) -> Result<()> {
    if rotation.iter().any(|v| !v.is_finite()) || !(-90.0..=90.0).contains(&rotation[1]) {
        return Err(super::registry::invalid(
            "rotation must be finite, with pitch -90..90",
        ));
    }
    Ok(())
}
