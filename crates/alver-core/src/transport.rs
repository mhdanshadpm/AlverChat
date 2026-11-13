//! Authenticated, forward-secret Noise transport for reliable LAN streams.

use std::io::{Read, Write};

use ed25519_dalek::VerifyingKey;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use snow::{params::NoiseParams, Builder, TransportState};

use crate::{identity::Identity, peer_id};

pub const MAX_TRANSPORT_FRAME_BYTES: usize = 64 * 1024;
const MAX_HANDSHAKE_FRAME_BYTES: usize = 1024;
const IDENTITY_CONTEXT: &[u8] = b"AlverChat Noise_XX_25519_ChaChaPoly_BLAKE2s v1";
const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteIdentity {
    pub peer_id: String,
    pub public_key: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct IdentityProof {
    v: u16,
    peer_id: String,
    public_key: [u8; 32],
    noise_static: Vec<u8>,
    signature: Vec<u8>,
}

pub struct SecureStream<S> {
    stream: S,
    transport: TransportState,
    remote: RemoteIdentity,
}

impl<S: Read + Write> SecureStream<S> {
    pub fn remote_identity(&self) -> &RemoteIdentity {
        &self.remote
    }

    pub fn send<T: Serialize>(&mut self, value: &T) -> Result<(), String> {
        let plaintext = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        if plaintext.len() > MAX_TRANSPORT_FRAME_BYTES - 16 {
            return Err("secure message exceeds the transport frame limit".into());
        }
        let mut encrypted = vec![0; plaintext.len() + 16];
        let size = self
            .transport
            .write_message(&plaintext, &mut encrypted)
            .map_err(|error| format!("secure message encryption failed: {error}"))?;
        encrypted.truncate(size);
        write_frame(&mut self.stream, &encrypted, MAX_TRANSPORT_FRAME_BYTES)
    }

    pub fn receive<T: DeserializeOwned>(&mut self) -> Result<T, String> {
        let encrypted = read_frame(&mut self.stream, MAX_TRANSPORT_FRAME_BYTES)?;
        let mut plaintext = vec![0; encrypted.len()];
        let size = self
            .transport
            .read_message(&encrypted, &mut plaintext)
            .map_err(|error| format!("secure message authentication failed: {error}"))?;
        plaintext.truncate(size);
        serde_json::from_slice(&plaintext).map_err(|error| error.to_string())
    }
}

pub fn initiator_handshake<S: Read + Write>(
    mut stream: S,
    identity: &Identity,
    expected: &RemoteIdentity,
) -> Result<SecureStream<S>, String> {
    let params = noise_params()?;
    let keypair = Builder::new(params.clone())
        .generate_keypair()
        .map_err(|error| error.to_string())?;
    let mut handshake = Builder::new(params)
        .local_private_key(&keypair.private)
        .map_err(|error| error.to_string())?
        .build_initiator()
        .map_err(|error| error.to_string())?;

    let mut output = vec![0; MAX_HANDSHAKE_FRAME_BYTES];
    let size = handshake
        .write_message(&[], &mut output)
        .map_err(|error| error.to_string())?;
    write_frame(&mut stream, &output[..size], MAX_HANDSHAKE_FRAME_BYTES)?;

    let incoming = read_frame(&mut stream, MAX_HANDSHAKE_FRAME_BYTES)?;
    let mut payload = vec![0; MAX_HANDSHAKE_FRAME_BYTES];
    let size = handshake
        .read_message(&incoming, &mut payload)
        .map_err(|error| format!("peer handshake failed: {error}"))?;
    payload.truncate(size);
    let remote = verify_identity_proof(
        &payload,
        handshake
            .get_remote_static()
            .ok_or("peer handshake did not provide a static key")?,
        Some(expected),
    )?;

    let proof = identity_proof(identity, &keypair.public)?;
    let size = handshake
        .write_message(&proof, &mut output)
        .map_err(|error| error.to_string())?;
    write_frame(&mut stream, &output[..size], MAX_HANDSHAKE_FRAME_BYTES)?;

    let transport = handshake
        .into_transport_mode()
        .map_err(|error| error.to_string())?;
    Ok(SecureStream {
        stream,
        transport,
        remote,
    })
}

pub fn responder_handshake<S: Read + Write>(
    mut stream: S,
    identity: &Identity,
) -> Result<SecureStream<S>, String> {
    let params = noise_params()?;
    let keypair = Builder::new(params.clone())
        .generate_keypair()
        .map_err(|error| error.to_string())?;
    let mut handshake = Builder::new(params)
        .local_private_key(&keypair.private)
        .map_err(|error| error.to_string())?
        .build_responder()
        .map_err(|error| error.to_string())?;

    let incoming = read_frame(&mut stream, MAX_HANDSHAKE_FRAME_BYTES)?;
    let mut payload = vec![0; MAX_HANDSHAKE_FRAME_BYTES];
    handshake
        .read_message(&incoming, &mut payload)
        .map_err(|error| format!("peer handshake failed: {error}"))?;

    let proof = identity_proof(identity, &keypair.public)?;
    let mut output = vec![0; MAX_HANDSHAKE_FRAME_BYTES];
    let size = handshake
        .write_message(&proof, &mut output)
        .map_err(|error| error.to_string())?;
    write_frame(&mut stream, &output[..size], MAX_HANDSHAKE_FRAME_BYTES)?;

    let incoming = read_frame(&mut stream, MAX_HANDSHAKE_FRAME_BYTES)?;
    let mut payload = vec![0; MAX_HANDSHAKE_FRAME_BYTES];
    let size = handshake
        .read_message(&incoming, &mut payload)
        .map_err(|error| format!("peer identity proof failed: {error}"))?;
    payload.truncate(size);
    let remote = verify_identity_proof(
        &payload,
        handshake
            .get_remote_static()
            .ok_or("peer handshake did not provide a static key")?,
        None,
    )?;

    let transport = handshake
        .into_transport_mode()
        .map_err(|error| error.to_string())?;
    Ok(SecureStream {
        stream,
        transport,
        remote,
    })
}

fn noise_params() -> Result<NoiseParams, String> {
    NOISE_PATTERN
        .parse()
        .map_err(|error: snow::Error| error.to_string())
}

fn identity_proof(identity: &Identity, noise_static: &[u8]) -> Result<Vec<u8>, String> {
    let public_key = identity.verifying_key().to_bytes();
    let peer_id = peer_id(&public_key);
    let signature = identity.sign(&proof_bytes(&peer_id, &public_key, noise_static));
    serde_json::to_vec(&IdentityProof {
        v: crate::PROTOCOL_VERSION,
        peer_id,
        public_key,
        noise_static: noise_static.to_vec(),
        signature: signature.to_bytes().to_vec(),
    })
    .map_err(|error| error.to_string())
}

fn verify_identity_proof(
    bytes: &[u8],
    noise_static: &[u8],
    expected: Option<&RemoteIdentity>,
) -> Result<RemoteIdentity, String> {
    let proof: IdentityProof = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if proof.v != crate::PROTOCOL_VERSION || proof.noise_static != noise_static {
        return Err("peer identity proof does not match the negotiated key".into());
    }
    if proof.peer_id != peer_id(&proof.public_key) {
        return Err("peer identity does not match its public key".into());
    }
    let key = VerifyingKey::from_bytes(&proof.public_key)
        .map_err(|error| format!("invalid peer public key: {error}"))?;
    let signature: [u8; 64] = proof
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| "peer identity signature has an invalid size")?;
    let signature = ed25519_dalek::Signature::from_bytes(&signature);
    if !Identity::verify(
        &key,
        &proof_bytes(&proof.peer_id, &proof.public_key, &proof.noise_static),
        &signature,
    ) {
        return Err("peer identity signature is invalid".into());
    }
    let remote = RemoteIdentity {
        peer_id: proof.peer_id,
        public_key: proof.public_key,
    };
    if let Some(expected) = expected {
        if &remote != expected {
            return Err("the peer identity does not match the signed LAN announcement".into());
        }
    }
    Ok(remote)
}

fn proof_bytes(peer_id: &str, public_key: &[u8; 32], noise_static: &[u8]) -> Vec<u8> {
    [
        IDENTITY_CONTEXT,
        peer_id.as_bytes(),
        public_key,
        noise_static,
    ]
    .concat()
}

fn write_frame<W: Write>(writer: &mut W, bytes: &[u8], max_size: usize) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > max_size || bytes.len() > u32::MAX as usize {
        return Err("transport frame has an invalid size".into());
    }
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|()| writer.write_all(bytes))
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

fn read_frame<R: Read>(reader: &mut R, max_size: usize) -> Result<Vec<u8>, String> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .map_err(|error| error.to_string())?;
    let size = u32::from_be_bytes(length) as usize;
    if size == 0 || size > max_size {
        return Err("transport frame has an invalid size".into());
    }
    let mut bytes = vec![0; size];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{TcpListener, TcpStream},
        thread,
    };

    fn stream_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn noise_xx_authenticates_both_identities_and_encrypts_payloads() {
        let alice = Identity::from_secret([21; 32]);
        let bob = Identity::from_secret([22; 32]);
        let (alice_socket, bob_socket) = stream_pair();
        let alice_expected = RemoteIdentity {
            peer_id: bob.peer_id(),
            public_key: bob.verifying_key().to_bytes(),
        };
        let bob_expected = RemoteIdentity {
            peer_id: alice.peer_id(),
            public_key: alice.verifying_key().to_bytes(),
        };

        let bob_thread = thread::spawn(move || {
            let mut stream = responder_handshake(bob_socket, &bob).unwrap();
            assert_eq!(stream.remote_identity(), &bob_expected);
            let received: serde_json::Value = stream.receive().unwrap();
            assert_eq!(received, serde_json::json!({"text":"hello"}));
            stream.send(&serde_json::json!({"ack":true})).unwrap();
        });
        let mut stream = initiator_handshake(alice_socket, &alice, &alice_expected).unwrap();
        assert_eq!(stream.remote_identity(), &alice_expected);
        stream.send(&serde_json::json!({"text":"hello"})).unwrap();
        let response: serde_json::Value = stream.receive().unwrap();
        assert_eq!(response, serde_json::json!({"ack":true}));
        bob_thread.join().unwrap();
    }

    #[test]
    fn initiator_rejects_an_identity_that_differs_from_discovery() {
        let alice = Identity::from_secret([23; 32]);
        let actual_bob = Identity::from_secret([24; 32]);
        let spoofed_bob = Identity::from_secret([25; 32]);
        let (alice_socket, bob_socket) = stream_pair();
        let expected = RemoteIdentity {
            peer_id: spoofed_bob.peer_id(),
            public_key: spoofed_bob.verifying_key().to_bytes(),
        };
        let server = thread::spawn(move || {
            let _ = responder_handshake(bob_socket, &actual_bob);
        });
        assert!(initiator_handshake(alice_socket, &alice, &expected).is_err());
        server.join().unwrap();
    }

    #[test]
    fn transport_frame_limits_reject_zero_and_oversized_frames() {
        assert!(read_frame(&mut &b"\0\0\0\0"[..], 8).is_err());
        assert!(read_frame(&mut &b"\0\0\0\x09"[..], 8).is_err());
        assert!(write_frame(&mut Vec::new(), &[1; 9], 8).is_err());
    }
}
