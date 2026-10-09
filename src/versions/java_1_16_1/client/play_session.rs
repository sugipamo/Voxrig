//! Protocol 736 session packet handlers.
//! Called only by the ordered dispatcher while its coherent-state gate is held.
use super::*;

impl Bot {
    pub(super) async fn receive_custom_payload(&self, p: &[u8]) -> Result<()> {
        let mut rest = p;
        let channel = get_string(&mut rest)?;
        if rest.len() > self.connection_options.max_custom_payload_bytes {
            bail!(
                "custom payload has {} bytes, limit is {}",
                rest.len(),
                self.connection_options.max_custom_payload_bytes
            );
        }
        let data = Arc::from(rest);
        if channel == "minecraft:brand" {
            let mut brand_data = rest;
            if let Ok(brand) = get_string(&mut brand_data) {
                **self.server_brand.write().await = Some(brand.clone());
                self.emit(Event::ServerBrand(brand));
            }
        }
        self.emit(Event::CustomPayload { channel, data });
        Ok(())
    }

    pub(super) async fn receive_disconnect_packet(&self, p: &[u8]) -> Result<bool> {
        let mut s = p;
        // Decode the terminal reason before committing lifecycle.
        // A truncated kick is malformed input and must remain
        // classifiable as unknown by the supervisor.
        let reason = get_string(&mut s)?;
        self.common_receipts.lock().await.disconnect_reason =
            Some(crate::client::ui::UiText::LegacyJson {
                json: reason.clone(),
            });
        self.physics.lock().await.record_disconnect();
        self.connection
            .mark_terminal(TerminalClassification::Disconnected)
            .await;
        self.emit(Event::Disconnected { reason });
        Ok(false)
    }

    pub(super) async fn receive_tags(&self, p: &[u8], packet_sequence: u64) -> Result<()> {
        let tags = parse_tags(p)?;
        self.common_receipts.lock().await.registries.receive_tags(
            p,
            packet_sequence,
            crate::MinecraftVersion::Java1_16_1,
        )?;
        **self.tags.write().await = tags;
        self.emit(Event::TagsUpdated);
        Ok(())
    }
}
