//! Offline inventory of the common API. This does not connect or prove live readiness.
use voxrig::MinecraftVersion;
use voxrig::client::{Capabilities, Feature};

fn main() -> anyhow::Result<()> {
    let versions = [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11];
    let adapters: Vec<_> = versions
        .into_iter()
        .map(|version| {
            let capabilities = Capabilities::for_version(version);
            let features: Vec<_> = Feature::ALL
                .iter()
                .map(|feature| {
                    serde_json::json!({
                        // Debug names are inventory labels, not stable wire identifiers.
                        "feature": format!("{feature:?}"),
                        "support": capabilities.support(*feature),
                    })
                })
                .collect();
            serde_json::json!({
                "version": version.name(),
                "protocol": version.protocol(),
                "features": features,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "scope": "static_common_api_support",
            "adapters": adapters,
        }))?
    );
    Ok(())
}
