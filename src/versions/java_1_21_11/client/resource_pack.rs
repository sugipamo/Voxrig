//! Headless protocol acknowledgement, shared by configuration and play.
use super::{Responses, State};
use crate::{client::resource_pack::ResourcePackRequest, protocol::put_varint};

// ServerboundResourcePackPacket.Action ordinals. No download or renderer runs.
const HEADLESS_STATUSES: [i32; 2] = [3, 0]; // ACCEPTED, SUCCESSFULLY_LOADED

pub(super) fn add(state: &mut State, payload: &[u8], output: i32) -> anyhow::Result<Responses> {
    anyhow::ensure!(
        payload.len() <= crate::protocol::MAX_PACKET_SIZE,
        "resource-pack request exceeds packet limit"
    );
    let mut r = super::Reader::new(payload);
    let uuid: [u8; 16] = r.take(16)?.try_into()?;
    let url = r.string()?;
    let hash = r.string()?;
    anyhow::ensure!(
        hash.encode_utf16().count() <= 40,
        "resource-pack hash exceeds 40 UTF-16 code units"
    );
    let forced = r.bool()?;
    let prompt = if r.bool()? {
        Some(crate::client::ui::UiText::NativeNbt {
            bytes: r.encoded_nbt()?,
        })
    } else {
        None
    };
    r.end()?;
    let responses = HEADLESS_STATUSES
        .map(|status| {
            let mut response = uuid.to_vec();
            put_varint(&mut response, status);
            (output, response)
        })
        .to_vec();
    state.resource_pack_request = Some(crate::client::received(
        ResourcePackRequest {
            uuid,
            url,
            hash,
            forced,
            prompt,
        },
        state.sequence,
    ));
    state.events.record(
        state.sequence,
        crate::client::EventKind::ResourcePackRequested { uuid },
    );
    Ok(responses)
}
