//! Native MathHelper indexed trigonometry (angles in radians).
pub(super) fn trig(angle: f32, cosine: bool) -> f32 {
    crate::client::survival::model::trig(crate::MinecraftVersion::Java1_21_11, angle, cosine)
}
