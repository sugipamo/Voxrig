//! Received resource-pack requests. Asset downloading and rendering are separate.
use super::{ObservedValue, ui::UiText};
use crate::Result;

/// One Java 1.21.11 resource-pack request, preserved without fetching its URL.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ResourcePackRequest {
    /// Server-supplied UUID, in network byte order; scoped to this connection.
    pub uuid: [u8; 16],
    /// Original URL. The client does not access it.
    pub url: String,
    /// Original SHA-1 text, including an empty or unverified value.
    pub hash: String,
    /// Whether the server requires acceptance to continue.
    pub forced: bool,
    /// Optional native text component, without rendering.
    pub prompt: Option<UiText>,
}

impl super::Client {
    /// Latest received Java 1.21.11 pack request, or `None` before any arrives.
    /// Readable during configuration and after disconnect. Removal and
    /// reconfiguration retain this historical receipt; it is not an active pack
    /// list, response-delivery receipt or proof of downloaded/applied assets.
    /// Only one request is retained, bounded by the 2 MiB packet limit; a newer
    /// request replaces it. Java 1.16.1 returns `ErrorKind::Unsupported`.
    pub async fn resource_pack_request(
        &self,
    ) -> Result<Option<ObservedValue<ResourcePackRequest>>> {
        super::dispatch!(&self.adapter, a => super::adapter::EventOps::resource_pack_request(a).await)
    }
}
