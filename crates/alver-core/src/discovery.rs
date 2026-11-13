//! mDNS service announcement and a signed LAN broadcast discovery fallback.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::{identity::Identity, peer_id};

pub const DISCOVERY_PORT: u16 = 47_986;
pub const SERVICE_TYPE: &str = "_alverchat._tcp.local.";
pub const MAX_BEACON_BYTES: usize = 1_024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerBeacon {
    pub v: u16,
    pub peer_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub public_key: [u8; 32],
    pub signature: Vec<u8>,
}

impl PeerBeacon {
    pub fn signed(identity: &Identity, name: String, host: String, port: u16) -> Self {
        let public_key = identity.verifying_key().to_bytes();
        let peer_id = peer_id(&public_key);
        let bytes = signed_bytes(PROTOCOL_VERSION, &peer_id, &name, &host, port, &public_key);
        Self {
            v: PROTOCOL_VERSION,
            peer_id,
            name,
            host,
            port,
            public_key,
            signature: identity.sign(&bytes).to_bytes().to_vec(),
        }
    }

    pub fn verify(&self) -> bool {
        if self.v != PROTOCOL_VERSION || self.peer_id != peer_id(&self.public_key) {
            return false;
        }
        let Ok(key) = VerifyingKey::from_bytes(&self.public_key) else {
            return false;
        };
        let Ok(signature_bytes) = <[u8; 64]>::try_from(self.signature.as_slice()) else {
            return false;
        };
        let signature = Signature::from_bytes(&signature_bytes);
        Identity::verify(
            &key,
            &signed_bytes(
                self.v,
                &self.peer_id,
                &self.name,
                &self.host,
                self.port,
                &self.public_key,
            ),
            &signature,
        )
    }
}

fn signed_bytes(v: u16, id: &str, name: &str, host: &str, port: u16, key: &[u8; 32]) -> Vec<u8> {
    [
        v.to_be_bytes().as_slice(),
        id.as_bytes(),
        name.as_bytes(),
        host.as_bytes(),
        port.to_be_bytes().as_slice(),
        key.as_slice(),
    ]
    .concat()
}

pub struct UdpDiscovery {
    socket: UdpSocket,
}

pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
}

impl MdnsDiscovery {
    pub fn new() -> Result<Self, String> {
        ServiceDaemon::new()
            .map(|daemon| Self { daemon })
            .map_err(|error| error.to_string())
    }

    pub fn publish(
        &self,
        peer_id: &str,
        display_name: &str,
        hostname: &str,
        addresses: &[IpAddr],
        port: u16,
    ) -> Result<(), String> {
        let suffix: String = peer_id.chars().take(12).collect();
        let instance = format!("alverchat-{suffix}");
        let name_property = ("name", display_name);
        let peer_property = ("peer_id", peer_id);
        let port_property = ("protocol", "1");
        let properties = [name_property, peer_property, port_property];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            hostname,
            addresses,
            port,
            &properties[..],
        )
        .map_err(|error| error.to_string())?
        .enable_addr_auto();
        self.daemon
            .register(service)
            .map_err(|error| error.to_string())
    }

    pub fn browse(&self) -> Result<mdns_sd::Receiver<ServiceEvent>, String> {
        self.daemon
            .browse(SERVICE_TYPE)
            .map_err(|error| error.to_string())
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.daemon
            .shutdown()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

impl UdpDiscovery {
    pub fn bind(address: SocketAddr) -> Result<Self, String> {
        let socket = UdpSocket::bind(address).map_err(|error| error.to_string())?;
        socket
            .set_broadcast(true)
            .map_err(|error| error.to_string())?;
        Ok(Self { socket })
    }

    pub fn announce(&self, beacon: &PeerBeacon, broadcast: SocketAddr) -> Result<(), String> {
        let bytes = serde_json::to_vec(beacon).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_BEACON_BYTES {
            return Err("discovery beacon exceeds the size limit".into());
        }
        self.socket
            .send_to(&bytes, broadcast)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn receive_once(&self, timeout: Duration) -> Result<Option<PeerBeacon>, String> {
        self.socket
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        let mut buffer = [0_u8; MAX_BEACON_BYTES + 1];
        match self.socket.recv_from(&mut buffer) {
            Ok((size, _)) if size <= MAX_BEACON_BYTES => {
                let beacon: PeerBeacon =
                    serde_json::from_slice(&buffer[..size]).map_err(|error| error.to_string())?;
                Ok(beacon.verify().then_some(beacon))
            }
            Ok(_) => Err("discovery beacon exceeds the size limit".into()),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

pub const PROTOCOL_VERSION: u16 = crate::PROTOCOL_VERSION;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_signature_covers_identity_and_endpoint() {
        let identity = Identity::from_secret([8; 32]);
        let mut beacon = PeerBeacon::signed(&identity, "Ari".into(), "192.168.1.10".into(), 45000);
        assert!(beacon.verify());
        beacon.port += 1;
        assert!(!beacon.verify());
    }

    #[test]
    fn malformed_beacon_key_or_signature_is_rejected() {
        let identity = Identity::from_secret([8; 32]);
        let mut beacon = PeerBeacon::signed(&identity, "Ari".into(), "127.0.0.1".into(), 45000);
        beacon.public_key = [0; 32];
        assert!(!beacon.verify());
    }
}
