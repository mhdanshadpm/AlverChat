use std::{
    collections::VecDeque,
    future::Future,
    io::{self, Read, Write},
    net::TcpStream,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

use alver_core::{identity::Identity, transport::MAX_TRANSPORT_FRAME_BYTES};
use ed25519_dalek::{pkcs8::EncodePrivateKey, SigningKey};
use futures_util::{SinkExt, StreamExt};
use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream as TokioTcpStream,
    runtime::Builder,
    sync::mpsc as tokio_mpsc,
    time::timeout,
};
use tokio_rustls::server::TlsAcceptor;
use tokio_tungstenite::{
    accept_async_with_config, client_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message as WsMessage},
    Connector, WebSocketStream,
};
use x509_parser::prelude::parse_x509_certificate;

const MAX_WS_MESSAGE: usize = MAX_TRANSPORT_FRAME_BYTES + 4;
const IO_TIMEOUT: Duration = Duration::from_secs(60);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct BlockingWebSocket {
    outgoing: tokio_mpsc::Sender<Vec<u8>>,
    incoming: mpsc::Receiver<Result<Vec<u8>, String>>,
    read_buffer: VecDeque<u8>,
    write_buffer: Vec<u8>,
}

impl Read for BlockingWebSocket {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        while self.read_buffer.is_empty() {
            let frame = self
                .incoming
                .recv_timeout(IO_TIMEOUT)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        io::Error::new(io::ErrorKind::TimedOut, "peer read timed out")
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed the connection")
                    }
                })?
                .map_err(|error| io::Error::new(io::ErrorKind::ConnectionAborted, error))?;
            self.read_buffer.extend(frame);
        }
        let count = output.len().min(self.read_buffer.len());
        for slot in &mut output[..count] {
            *slot = self
                .read_buffer
                .pop_front()
                .expect("buffer length was checked");
        }
        Ok(count)
    }
}

impl Write for BlockingWebSocket {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.write_buffer.extend_from_slice(input);
        loop {
            if self.write_buffer.len() < 4 {
                break;
            }
            let size =
                u32::from_be_bytes(self.write_buffer[..4].try_into().expect("four bytes")) as usize;
            if size == 0 || size > MAX_TRANSPORT_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid secure frame size",
                ));
            }
            let total = size + 4;
            if self.write_buffer.len() < total {
                break;
            }
            let frame = self.write_buffer.drain(..total).collect();
            self.outgoing.blocking_send(frame).map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "WebSocket sender has closed")
            })?;
        }
        if self.write_buffer.len() > MAX_WS_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "secure frame exceeds WebSocket limit",
            ));
        }
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn accept(
    stream: TcpStream,
    config: Arc<rustls::ServerConfig>,
) -> Result<BlockingWebSocket, String> {
    start_driver(move || async move {
        let stream = to_tokio_stream(stream)?;
        let tls = timeout(HANDSHAKE_TIMEOUT, TlsAcceptor::from(config).accept(stream))
            .await
            .map_err(|_| "TLS handshake timed out".to_string())?
            .map_err(|error| format!("TLS handshake failed: {error}"))?;
        timeout(
            HANDSHAKE_TIMEOUT,
            accept_async_with_config(tls, Some(websocket_config())),
        )
        .await
        .map_err(|_| "WebSocket handshake timed out".to_string())?
        .map_err(|error| format!("WebSocket handshake failed: {error}"))
    })
}

pub fn connect(
    stream: TcpStream,
    peer_id: &str,
    expected_key: [u8; 32],
) -> Result<BlockingWebSocket, String> {
    let config = client_config(expected_key)?;
    let request = format!("wss://localhost/peer/{peer_id}")
        .into_client_request()
        .map_err(|error| format!("invalid secure WebSocket request: {error}"))?;
    start_driver(move || async move {
        let stream = to_tokio_stream(stream)?;
        let connector = Connector::Rustls(config);
        timeout(
            HANDSHAKE_TIMEOUT,
            client_async_tls_with_config(
                request,
                stream,
                Some(websocket_config()),
                Some(connector),
            ),
        )
        .await
        .map_err(|_| "secure WebSocket connection timed out".to_string())?
        .map(|(ws, _)| ws)
        .map_err(|error| format!("secure WebSocket connection failed: {error}"))
    })
}

fn to_tokio_stream(stream: TcpStream) -> Result<TokioTcpStream, String> {
    stream
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    TokioTcpStream::from_std(stream).map_err(|error| error.to_string())
}

fn websocket_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_WS_MESSAGE);
    config.max_frame_size = Some(MAX_WS_MESSAGE);
    config
}

pub fn server_config(identity: &Identity) -> Result<Arc<rustls::ServerConfig>, String> {
    let key = SigningKey::from_bytes(&identity.secret_bytes());
    let encoded = key.to_pkcs8_der().map_err(|error| error.to_string())?;
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(encoded.as_bytes().to_vec()));
    let signing_key = KeyPair::from_der_and_sign_algo(&key_der, &PKCS_ED25519)
        .map_err(|error| format!("could not derive TLS key from peer identity: {error}"))?;
    let cert = CertificateParams::new(vec!["localhost".to_owned()])
        .map_err(|error| error.to_string())?
        .self_signed(&signing_key)
        .map_err(|error| format!("could not create self-signed peer certificate: {error}"))?;
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| format!("could not configure TLS 1.3: {error}"))?
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private_key)
        .map_err(|error| format!("could not configure TLS identity: {error}"))?;
    Ok(Arc::new(config))
}

fn client_config(expected_key: [u8; 32]) -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PeerCertificateVerifier {
        expected_key,
        provider: provider.clone(),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| format!("could not configure TLS 1.3: {error}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alver_core::transport::{initiator_handshake, responder_handshake, RemoteIdentity};
    use std::net::TcpListener;

    #[test]
    fn tls13_websocket_authenticates_the_discovered_identity_and_exchanges_data() {
        let client_identity = Identity::from_secret([21; 32]);
        let server_identity = Identity::from_secret([42; 32]);
        let server_config = server_config(&server_identity).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server_public_key = server_identity.verifying_key().to_bytes();
        let server_id = server_identity.peer_id();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let websocket = accept(stream, server_config).unwrap();
            let mut session = responder_handshake(websocket, &server_identity).unwrap();
            let message: String = session.receive().unwrap();
            assert_eq!(message, "hello over TLS WebSocket");
            session.send(&"authenticated reply".to_owned()).unwrap();
        });

        let stream = TcpStream::connect(address).unwrap();
        let websocket = connect(stream, &server_id, server_public_key).unwrap();
        let expected = RemoteIdentity {
            peer_id: server_id,
            public_key: server_public_key,
        };
        let mut session = initiator_handshake(websocket, &client_identity, &expected).unwrap();
        session
            .send(&"hello over TLS WebSocket".to_owned())
            .unwrap();
        let reply: String = session.receive().unwrap();
        assert_eq!(reply, "authenticated reply");
        server.join().unwrap();
    }

    #[test]
    fn websocket_tls_rejects_a_certificate_with_the_wrong_peer_key() {
        let server_identity = Identity::from_secret([51; 32]);
        let server_config = server_config(&server_identity).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            assert!(accept(stream, server_config).is_err());
        });
        let stream = TcpStream::connect(address).unwrap();
        assert!(connect(stream, "wrong-peer", [99; 32]).is_err());
        server.join().unwrap();
    }
}

#[derive(Debug)]
struct PeerCertificateVerifier {
    expected_key: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PeerCertificateVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let (_, parsed) = parse_x509_certificate(cert.as_ref())
            .map_err(|_| TlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
        if parsed
            .tbs_certificate
            .subject_pki
            .subject_public_key
            .data
            .as_ref()
            != self.expected_key.as_slice()
        {
            return Err(TlsError::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("TLS 1.2 is disabled".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn start_driver<S, F, Fut>(connect: F) -> Result<BlockingWebSocket, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<WebSocketStream<S>, String>> + Send + 'static,
{
    let (outgoing_tx, mut outgoing_rx) = tokio_mpsc::channel::<Vec<u8>>(16);
    let (incoming_tx, incoming_rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(16);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    thread::Builder::new()
        .name("alverchat-wss-session".into())
        .spawn(move || {
            let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
                let _ = ready_tx.send(Err("could not start WebSocket session runtime".into()));
                return;
            };
            runtime.block_on(async move {
                let mut websocket = match connect().await {
                    Ok(websocket) => {
                        let _ = ready_tx.send(Ok(()));
                        websocket
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        let _ = incoming_tx.send(Err(error));
                        return;
                    }
                };
                loop {
                    tokio::select! {
                        outgoing = outgoing_rx.recv() => match outgoing {
                            Some(bytes) => if let Err(error) = websocket.send(WsMessage::Binary(bytes.into())).await {
                                let _ = incoming_tx.send(Err(error.to_string()));
                                break;
                            },
                            None => { let _ = websocket.close(None).await; break; }
                        },
                        incoming = websocket.next() => match incoming {
                            Some(Ok(WsMessage::Binary(bytes))) => {
                                if bytes.len() > MAX_WS_MESSAGE {
                                    let _ = incoming_tx.send(Err("oversized WebSocket frame".into()));
                                    break;
                                }
                                if incoming_tx.send(Ok(bytes.to_vec())).is_err() { break; }
                            }
                            Some(Ok(WsMessage::Ping(bytes))) => {
                                if let Err(error) = websocket.send(WsMessage::Pong(bytes)).await {
                                    let _ = incoming_tx.send(Err(error.to_string())); break;
                                }
                            }
                            Some(Ok(WsMessage::Close(_))) | None => break,
                            Some(Ok(WsMessage::Pong(_))) | Some(Ok(WsMessage::Frame(_))) => {}
                            Some(Ok(WsMessage::Text(_))) => {
                                let _ = incoming_tx.send(Err("text WebSocket frames are not accepted".into())); break;
                            }
                            Some(Err(error)) => {
                                let _ = incoming_tx.send(Err(error.to_string())); break;
                            }
                        }
                    }
                }
            });
        })
        .map_err(|error| format!("could not start WebSocket session: {error}"))?;
    ready_rx
        .recv_timeout(HANDSHAKE_TIMEOUT + Duration::from_secs(2))
        .map_err(|_| "secure WebSocket session did not become ready".to_string())??;
    Ok(BlockingWebSocket {
        outgoing: outgoing_tx,
        incoming: incoming_rx,
        read_buffer: VecDeque::new(),
        write_buffer: Vec::new(),
    })
}
