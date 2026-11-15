use serde_json::Value;

use crate::{Envelope, PROTOCOL_VERSION};

pub const MAX_CONTROL_FRAME_BYTES: usize = 1_048_576;
/// One file chunk, and the largest body a `file_chunk` frame carries.
///
/// A chunk travels base64-encoded inside a JSON envelope, so the whole frame has to fit
/// the transport's single 64 KiB frame: 40 KiB of bytes expand to 54,616 base64
/// characters, which leaves room for the envelope and the AEAD tag.
pub const MAX_FILE_CHUNK_BYTES: usize = 40 * 1024;

/// The envelope types this build's framing layer accepts.
///
/// This is the declared table itself, not a second list of the same names. The copy was
/// the reason a frame could be on the wire and off the table at the same time.
const KNOWN_TYPES: &[&str] = crate::payloads::ALL_TYPES;

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

    mod properties {
        use super::super::*;
        use super::encoded;
        use proptest::prelude::*;

        fn any_kind() -> impl Strategy<Value = &'static str> {
            proptest::sample::select(KNOWN_TYPES.to_vec())
        }

        proptest! {
            /// Anything a hostile peer can put on the wire must come back as a
            /// result — never a panic, an abort or an unbounded allocation.
            #[test]
            fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
                let _ = decode_control_frame(&bytes);
            }

            #[test]
            fn known_envelopes_round_trip(
                kind in any_kind(),
                from in ".{0,64}",
                to in ".{0,64}",
                physical_ms in any::<u64>(),
                logical in any::<u32>(),
                text in ".{0,128}",
            ) {
                let id = uuid::Uuid::now_v7();
                let envelope = Envelope {
                    v: PROTOCOL_VERSION,
                    id,
                    kind: kind.to_string(),
                    from: from.clone(),
                    to: to.clone(),
                    hlc: crate::Hlc { physical_ms, logical },
                    payload: serde_json::json!({ "text": text }),
                };
                let bytes = serde_json::to_vec(&envelope).unwrap();
                let decoded = decode_control_frame(&bytes).unwrap().expect("known kind decodes");
                prop_assert_eq!(decoded.id, id);
                prop_assert_eq!(&decoded.kind, kind);
                prop_assert_eq!(decoded.from, from);
                prop_assert_eq!(decoded.to, to);
                prop_assert_eq!(decoded.hlc.physical_ms, physical_ms);
                prop_assert_eq!(decoded.hlc.logical, logical);
            }

            #[test]
            fn unsupported_versions_are_rejected(v in any::<u16>().prop_filter("not the protocol version", |v| *v != PROTOCOL_VERSION)) {
                let mut value: serde_json::Value = serde_json::from_slice(&encoded("message")).unwrap();
                value["v"] = serde_json::json!(v);
                let bytes = serde_json::to_vec(&value).unwrap();
                prop_assert_eq!(decode_control_frame(&bytes), Err(ProtocolError::UnsupportedVersion(v)));
            }
        }
    }
}
