//! Java 1.21.11 wire protocol (774), registries and client behavior.
mod client;
#[allow(dead_code)]
mod ids;
mod wire;
mod world;
pub(crate) use client::Bot;
pub use client::{PacketRecord, PacketTrace};

fn registry() -> &'static crate::block_state::StateRegistry {
    static REGISTRY: std::sync::OnceLock<crate::block_state::StateRegistry> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        crate::block_state::StateRegistry::parse(include_str!(
            "../../../data/java_1_21_11/blocks.json"
        ))
        .expect("valid bundled 1.21.11 registry")
    })
}

/// Interprets an ID only in the Java 1.21.11 registry.
pub fn native_state(id: i32) -> crate::Result<crate::NativeBlockState> {
    registry().decode(id)
}

fn validate_state_id(id: i32) -> anyhow::Result<()> {
    registry().validate_id(id)?;
    Ok(())
}
