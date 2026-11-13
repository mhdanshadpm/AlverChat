use serde_json::Value;

use crate::{Envelope, PROTOCOL_VERSION};

pub const MAX_CONTROL_FRAME_BYTES: usize = 1_048_576;
/// One file chunk, and the largest body a `file_chunk` frame carries.
///
/// A chunk travels base64-encoded inside a JSON envelope, so the whole frame has to fit
/// the transport's single 64 KiB frame: 40 KiB of bytes expand to 54,616 base64
/// characters, which leaves room for the envelope and the AEAD tag.
pub const MAX_FILE_CHUNK_BYTES: usize = 40 * 1024;

const KNOWN_TYPES: &[&str] = &[
    "hello",
    "hello_ack",
    "presence",
    "profile_update",
    "avatar_request",
    "avatar_data",
    "message",
    "message_edit",
    "message_delete",
    "reaction",
    "receipt",
    "typing",
    "pin_update",
    "group_create",
    "group_update",
    "group_member_change",
    "group_icon_update",
    "announcement",
    "announcement_ack",
    "file_offer",
    "file_accept",
    "file_chunk",
    "file_ack",
    "file_cancel",
    "call_offer",
    "call_answer",
    "ice_candidate",
    "call_end",
    "call_reject",
    "call_busy",
    "sync_request",
    "ping",
    "pong",
    "bye",
];

#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolError {
    FrameTooLarge,
    InvalidEnvelope(String),
    UnsupportedVersion(u16),
}

pub fn decode_control_frame(bytes: &[u8]) -> Result<Option<Envelope<Value>>, ProtocolError> {
    if bytes.len() > MAX_CONTROL_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let envelope: Envelope<Value> = serde_json::from_slice(bytes)
        .map_err(|error| ProtocolError::InvalidEnvelope(error.to_string()))?;
    if envelope.v != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(envelope.v));
    }
    Ok(KNOWN_TYPES
        .contains(&envelope.kind.as_str())
        .then_some(envelope))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(kind: &str) -> Vec<u8> {
        serde_json::to_vec(&Envelope {
            v: PROTOCOL_VERSION,
            id: uuid::Uuid::now_v7(),
            kind: kind.into(),
            from: "peer-a".into(),
            to: "chat-a".into(),
            hlc: crate::Hlc::default(),
            payload: serde_json::json!({"text":"hi"}),
        })
        .unwrap()
    }

    #[test]
    fn valid_known_message_decodes() {
        assert!(decode_control_frame(&encoded("message")).unwrap().is_some());
    }

    #[test]
    fn unknown_message_kind_is_ignored() {
        assert_eq!(
            decode_control_frame(&encoded("future_feature")).unwrap(),
            None
        );
    }

    #[test]
    fn malformed_and_oversized_frames_are_rejected() {
        assert!(matches!(
            decode_control_frame(b"{bad"),
            Err(ProtocolError::InvalidEnvelope(_))
        ));
        assert_eq!(
            decode_control_frame(&vec![b' '; MAX_CONTROL_FRAME_BYTES + 1]),
            Err(ProtocolError::FrameTooLarge)
        );
    }
}
