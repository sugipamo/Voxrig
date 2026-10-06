//! Original login profile receipts, independent of play packet ordinals.
use crate::{Result, protocol::get_string};

#[derive(Clone)]
pub(crate) struct LoginProfile {
    pub uuid: [u8; 16],
    pub name: String,
}

/// The original 1.16.1 GameProfile packet writes four big-endian i32 UUID
/// words (16 bytes), followed by the profile name. No properties field follows.
pub(crate) fn legacy_profile(payload: &[u8], expected: &str) -> Result<LoginProfile> {
    let uuid = payload
        .get(..16)
        .ok_or_else(|| super::registry::invalid("truncated login UUID"))?
        .try_into()
        .expect("16-byte UUID");
    let mut remaining = &payload[16..];
    let name = get_string(&mut remaining)?;
    if name != expected || !remaining.is_empty() {
        return Err(super::registry::invalid(
            "login profile differs from the requested name or has trailing fields",
        ));
    }
    Ok(LoginProfile { uuid, name })
}

#[cfg(test)]
pub(crate) fn test_legacy_success(mut login_request: &[u8]) -> Vec<u8> {
    let name = get_string(&mut login_request).unwrap();
    let mut payload = vec![3; 16];
    crate::protocol::put_string(&mut payload, &name);
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_vanilla_login_uuid_matches_the_independently_saved_player() {
        // Captured 1.16.1 LOGIN_SUCCESS from trial-1.16.1-2617c684. Its UUID
        // independently names world/playerdata/7af52aef-398f-3a62-b13e-88adcd0ea96d.dat.
        let payload =
            hex::decode("7af52aef398f3a62b13e88adcd0ea96d0c556e696669656450726f6265").unwrap();
        let received = legacy_profile(&payload, "UnifiedProbe").unwrap();
        assert_eq!(
            received.uuid,
            [
                0x7a, 0xf5, 0x2a, 0xef, 0x39, 0x8f, 0x3a, 0x62, 0xb1, 0x3e, 0x88, 0xad, 0xcd, 0x0e,
                0xa9, 0x6d
            ]
        );
    }
    #[test]
    fn login_profile_preserves_received_uuid_and_rejects_missing_or_foreign_fields() {
        let mut request = Vec::new();
        crate::protocol::put_string(&mut request, "ProfileProbe");
        let mut success = test_legacy_success(&request);
        // Four original network-order words, including negative native i32 values.
        let uuid = [
            0x80, 0, 0, 1, 0x7f, 0xff, 0xff, 0xff, 0, 0, 0, 2, 0xff, 0xff, 0xff, 0xfd,
        ];
        success[..16].copy_from_slice(&uuid);
        let actual = legacy_profile(&success, "ProfileProbe").unwrap();
        assert_eq!(actual.uuid, uuid);
        assert_eq!(actual.name, "ProfileProbe");
        for length in [0, 15, 16, success.len() - 1] {
            assert!(legacy_profile(&success[..length], "ProfileProbe").is_err());
        }
        assert!(legacy_profile(&success, "OtherProfile").is_err());
        success.push(0);
        assert!(legacy_profile(&success, "ProfileProbe").is_err());
    }
}
