//! mDNS service announcement and a signed LAN broadcast discovery fallback.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};
use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::{identity::Identity, peer_id};

pub const DISCOVERY_PORT: u16 = 47_986;
pub const SERVICE_TYPE: &str = "_alverchat._tcp.local.";
pub const MAX_BEACON_BYTES: usize = 1_024;

/// The instance this peer registers under. `unpublish` has to name the same instance,
/// or asking not to be visible would take somebody else's beacon down.
fn beacon_instance(beacon: &PeerBeacon) -> String {
    let suffix: String = beacon.peer_id.chars().take(12).collect();
    format!("alverchat-{suffix}")
}

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
        if self.v != PROTOCOL_VERSION
            || self.peer_id != peer_id(&self.public_key)
            || self.name.is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || self.host.is_empty()
            || self.port == 0
        {
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

    pub fn from_mdns(service: &ResolvedService) -> Option<Self> {
        let properties = service.get_properties();
        let version = properties.get_property_val_str("protocol")?.parse().ok()?;
        let peer_id = properties.get_property_val_str("peer_id")?.to_owned();
        let name = properties.get_property_val_str("name")?.to_owned();
        let public_key = decode_hex::<32>(properties.get_property_val_str("public_key")?)?;
        let signature = decode_hex::<64>(properties.get_property_val_str("signature")?)?.to_vec();
        let beacon = Self {
            v: version,
            peer_id,
            name,
            host: service.get_hostname().to_owned(),
            port: service.get_port(),
            public_key,
            signature,
        };
        beacon.verify().then_some(beacon)
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut decoded = [0; N];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = (pair[0] as char).to_digit(16)? as u8;
        let low = (pair[1] as char).to_digit(16)? as u8;
        decoded[index] = (high << 4) | low;
    }
    Some(decoded)
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

#[derive(Clone)]
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
}

impl MdnsDiscovery {
    pub fn new() -> Result<Self, String> {
        ServiceDaemon::new()
            .map(|daemon| Self { daemon })
            .map_err(|error| error.to_string())
    }

    pub fn publish(&self, beacon: &PeerBeacon, addresses: &[IpAddr]) -> Result<(), String> {
        if !beacon.verify() || beacon.name.len() > 128 {
            return Err("invalid signed LAN announcement".into());
        }
        let instance = beacon_instance(beacon);
        let public_key = encode_hex(&beacon.public_key);
        let signature = encode_hex(&beacon.signature);
        let version = beacon.v.to_string();
        let properties = [
            ("name", beacon.name.as_str()),
            ("peer_id", beacon.peer_id.as_str()),
            ("protocol", version.as_str()),
            ("public_key", public_key.as_str()),
            ("signature", signature.as_str()),
        ];
        let mut service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &beacon.host,
            addresses,
            beacon.port,
            &properties[..],
        )
        .map_err(|error| error.to_string())?
        .enable_addr_auto();
        // Settings → Network adapter: name one interface and the beacon leaves the others
        // alone. Empty means automatic, which is the daemon's own view of the host.
        if let Some(address) = addresses.first() {
            service.set_interfaces(vec![IfKind::Addr(*address)]);
        }
        self.daemon
            .register(service)
            .map_err(|error| error.to_string())
    }

    /// Takes the beacon back down, so a machine that asked not to be visible stops
    /// answering the browse that would have listed it.
    pub fn unpublish(&self, beacon: &PeerBeacon) -> Result<(), String> {
        let fullname = format!("{}.{}", beacon_instance(beacon), SERVICE_TYPE);
        self.daemon
            .unregister(&fullname)
            .map(|_status| ())
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

    /// Where this listener is actually reachable, which is how a caller finds the port
    /// when it asked to be bound to `:0`.
    pub fn local_addr(&self) -> Result<SocketAddr, String> {
        self.socket.local_addr().map_err(|error| error.to_string())
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

    /// The next beacon heard, or `None` when nothing worth keeping arrived before the
    /// timeout.
    ///
    /// A discovery port is open to the whole subnet, so this also hears other software
    /// and truncated or tampered datagrams. None of that is a reason to stop listening:
    /// anything that does not re-hash to a signed beacon of this shape is ignored.
    pub fn receive_once(&self, timeout: Duration) -> Result<Option<PeerBeacon>, String> {
        self.socket
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        let mut buffer = [0_u8; MAX_BEACON_BYTES];
        match self.socket.recv_from(&mut buffer) {
            Ok((size, _)) => {
                let Ok(beacon) = serde_json::from_slice::<PeerBeacon>(&buffer[..size]) else {
                    return Ok(None);
                };
                Ok(beacon.verify().then_some(beacon))
            }
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
    fn each_peer_gets_its_own_beacon_instance() {
        // Taking our own beacon down must not take somebody else's down with it.
        let first = PeerBeacon::signed(
            &Identity::from_secret([8; 32]),
            "Ari".into(),
            "host.local.".into(),
            45000,
        );
        let second = PeerBeacon::signed(
            &Identity::from_secret([9; 32]),
            "Bo".into(),
            "host.local.".into(),
            45000,
        );
        assert_eq!(
            beacon_instance(&first),
            format!("alverchat-{}", &first.peer_id[..12])
        );
        assert_ne!(beacon_instance(&first), beacon_instance(&second));
    }

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

        let mut beacon = PeerBeacon::signed(&identity, "Ari".into(), "127.0.0.1".into(), 45000);
        beacon.name = "A".repeat(129);
        assert!(!beacon.verify());
    }

    #[test]
    fn hex_encoding_round_trips_and_rejects_malformed_values() {
        let bytes = [0x00, 0x7f, 0xa5, 0xff];
        assert_eq!(decode_hex::<4>(&encode_hex(&bytes)), Some(bytes));
        assert_eq!(decode_hex::<4>("007fa5"), None);
        assert_eq!(decode_hex::<4>("007fa5xz"), None);
    }

    fn loopback_listener() -> UdpDiscovery {
        UdpDiscovery::bind("127.0.0.1:0".parse().expect("loopback wildcard"))
            .expect("binding a loopback discovery socket must work")
    }

    #[test]
    fn a_beacon_that_arrives_over_udp_is_the_one_that_was_signed() {
        // Announcer and listener are both the production type, so what the fallback writes
        // is what the fallback reads back.
        let listener = loopback_listener();
        let address = listener
            .local_addr()
            .expect("a bound socket has an address");
        let beacon = PeerBeacon::signed(
            &Identity::from_secret([8; 32]),
            "Ari".into(),
            "192.168.1.20".into(),
            45_123,
        );
        loopback_listener()
            .announce(&beacon, address)
            .expect("loopback announce");
        assert_eq!(
            listener
                .receive_once(Duration::from_secs(1))
                .expect("listening"),
            Some(beacon)
        );
    }

    #[test]
    fn a_stray_datagram_is_ignored_without_stopping_the_listener() {
        // The discovery port is open to the whole subnet, so other traffic landing on it
        // must cost nothing and must not end the loop that reads it.
        let listener = loopback_listener();
        let address = listener.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(b"not a beacon at all", address)
            .expect("loopback send");
        assert_eq!(
            listener
                .receive_once(Duration::from_millis(300))
                .expect("listening"),
            None
        );

        let beacon = PeerBeacon::signed(
            &Identity::from_secret([9; 32]),
            "Bo".into(),
            "192.168.1.21".into(),
            45_123,
        );
        sender
            .send_to(&serde_json::to_vec(&beacon).unwrap(), address)
            .expect("loopback send");
        assert_eq!(
            listener
                .receive_once(Duration::from_secs(1))
                .expect("still listening"),
            Some(beacon)
        );
    }

    #[test]
    fn a_tampered_or_oversize_datagram_is_never_accepted_as_a_beacon() {
        let listener = loopback_listener();
        let address = listener.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();

        let mut beacon = PeerBeacon::signed(
            &Identity::from_secret([10; 32]),
            "Cy".into(),
            "192.168.1.22".into(),
            45_123,
        );
        // The endpoint is inside the signature, so a datagram claiming a different
        // address for the same identity is refused rather than dialled.
        beacon.host = "192.168.1.99".into();
        sender
            .send_to(&serde_json::to_vec(&beacon).unwrap(), address)
            .expect("loopback send");
        assert_eq!(
            listener
                .receive_once(Duration::from_millis(300))
                .expect("listening"),
            None
        );

        sender
            .send_to(&[7_u8; MAX_BEACON_BYTES * 4], address)
            .expect("loopback send");
        assert_eq!(
            listener
                .receive_once(Duration::from_millis(300))
                .expect("still listening"),
            None
        );
    }
}
