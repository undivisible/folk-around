use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use folk_mcp::{ToolTable, handle_message};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::{Value, json};
use sha2::Sha256;
use thiserror::Error;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use url::Url;

const RELAY_AD: &[u8] = b"folk-around p2p relay v1";
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_SSE_CLIENTS: usize = 32;
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
static SSE_CLIENTS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    WebSocket(#[from] tungstenite::Error),
    #[error("{0}")]
    Url(#[from] url::ParseError),
    #[error("invalid signal url")]
    InvalidSignalUrl,
    #[error("invalid peer identity")]
    InvalidPeerIdentity,
    #[error("handshake required")]
    HandshakeRequired,
    #[error("invalid encrypted payload")]
    InvalidEncryptedPayload,
    #[error("crypto error")]
    Crypto,
}

pub fn run_stdio(verbose: bool, table: Arc<ToolTable>) -> Result<(), TransportError> {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();
    while let Some(body) = read_stdio_message(&mut reader)? {
        let msg = match serde_json::from_slice::<Value>(&body) {
            Ok(msg) => msg,
            Err(err) => {
                if verbose {
                    log_status(&format!("json error: {err}"));
                }
                continue;
            }
        };
        if let Some(out) = handle_message(verbose, &table, msg)? {
            write!(stdout, "Content-Length: {}\r\n\r\n{}", out.len(), out)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

pub fn run_http(verbose: bool, table: Arc<ToolTable>, port: u16) -> Result<(), TransportError> {
    let listener = TcpListener::bind(http_bind_addr(port))?;
    if verbose {
        log_status(&format!("HTTP listening on http://127.0.0.1:{port}/"));
    }
    for stream in listener.incoming() {
        let table = Arc::clone(&table);
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    let _ = handle_http_client(stream, verbose, table);
                });
            }
            Err(err) if verbose => log_status(&format!("HTTP accept error: {err}")),
            Err(_) => {}
        }
    }
    Ok(())
}

pub fn start_p2p(verbose: bool, table: Arc<ToolTable>, signal_url: String, room: String) {
    thread::spawn(move || {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut manager = P2PManager::new(verbose, table, signal_url, room);
        loop {
            if let Err(err) = manager.join_signal_room()
                && manager.verbose
            {
                log_status(&format!("signaling error: {err}"));
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn read_stdio_message(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, TransportError> {
    let mut first_line = String::new();
    if reader.read_line(&mut first_line)? == 0 {
        return Ok(None);
    }
    let trimmed = first_line.trim();
    if trimmed.is_empty() {
        return read_stdio_message(reader);
    }
    if !trimmed.to_ascii_lowercase().starts_with("content-length:") {
        return Ok(Some(trimmed.as_bytes().to_vec()));
    }
    let len = trimmed["content-length:".len()..]
        .trim()
        .parse::<usize>()
        .unwrap_or(0);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            break;
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

fn handle_http_client(
    mut stream: TcpStream,
    verbose: bool,
    table: Arc<ToolTable>,
) -> Result<(), TransportError> {
    stream.set_read_timeout(Some(HTTP_READ_TIMEOUT))?;
    stream.set_write_timeout(Some(HTTP_WRITE_TIMEOUT))?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut expected_len = None;
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if expected_len.is_none()
            && let Some((offset, len)) = header_end(&buffer)
        {
            let body_len = content_length(&buffer[..offset]);
            if body_len > MAX_HTTP_BODY_BYTES {
                send_response(
                    &mut stream,
                    413,
                    "Payload Too Large",
                    "text/plain",
                    b"payload too large",
                )?;
                return Ok(());
            }
            expected_len = Some(offset + len + body_len);
        }
        if let Some(len) = expected_len {
            if buffer.len() >= len {
                break;
            }
        } else if buffer.len() > MAX_HTTP_HEADER_BYTES {
            send_response(
                &mut stream,
                400,
                "Bad Request",
                "text/plain",
                b"bad request",
            )?;
            return Ok(());
        }
    }

    let request = String::from_utf8_lossy(&buffer);
    let mut lines = request.lines();
    let request_line = lines.next().unwrap_or("").trim();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    match (method, path) {
        ("OPTIONS", _) => send_response(&mut stream, 204, "No Content", "text/plain", b"")?,
        ("GET", "/health") => send_response(&mut stream, 200, "OK", "text/plain", b"ok")?,
        ("GET", "/sse") => send_sse(stream)?,
        ("POST", "/message") => handle_http_post(stream, verbose, table, &buffer)?,
        _ => send_response(&mut stream, 404, "Not Found", "text/plain", b"not found")?,
    }
    Ok(())
}

fn handle_http_post(
    mut stream: TcpStream,
    verbose: bool,
    table: Arc<ToolTable>,
    request: &[u8],
) -> Result<(), TransportError> {
    let Some((offset, len)) = header_end(request) else {
        send_response(
            &mut stream,
            400,
            "Bad Request",
            "text/plain",
            b"missing headers",
        )?;
        return Ok(());
    };
    let body_start = offset + len;
    let body_len = content_length(&request[..offset]);
    if body_len > MAX_HTTP_BODY_BYTES {
        send_response(
            &mut stream,
            413,
            "Payload Too Large",
            "text/plain",
            b"payload too large",
        )?;
        return Ok(());
    }
    if body_len == 0 || body_start + body_len > request.len() {
        send_response(
            &mut stream,
            400,
            "Bad Request",
            "text/plain",
            b"missing body",
        )?;
        return Ok(());
    }
    let msg = match serde_json::from_slice::<Value>(&request[body_start..body_start + body_len]) {
        Ok(msg) => msg,
        Err(_) => {
            send_response(
                &mut stream,
                400,
                "Bad Request",
                "text/plain",
                b"invalid json",
            )?;
            return Ok(());
        }
    };
    match handle_message(verbose, &table, msg)? {
        Some(out) => send_response(&mut stream, 200, "OK", "application/json", out.as_bytes())?,
        None => send_response(&mut stream, 202, "Accepted", "text/plain", b"accepted")?,
    }
    Ok(())
}

fn send_sse(mut stream: TcpStream) -> Result<(), TransportError> {
    let Some(_slot) = SseSlot::new() else {
        send_response(
            &mut stream,
            503,
            "Service Unavailable",
            "text/plain",
            b"too many sse clients",
        )?;
        return Ok(());
    };
    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\naccess-control-allow-origin: *\r\naccess-control-allow-headers: content-type\r\naccess-control-allow-methods: GET, POST, OPTIONS\r\nconnection: keep-alive\r\n\r\n")?;
    stream.write_all(b"event: endpoint\ndata: /message\n\n")?;
    stream.flush()?;
    loop {
        thread::sleep(Duration::from_secs(15));
        if stream.write_all(b": keepalive\n\n").is_err() {
            return Ok(());
        }
        if stream.flush().is_err() {
            return Ok(());
        }
    }
}

fn send_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), TransportError> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\naccess-control-allow-origin: *\r\naccess-control-allow-headers: content-type\r\naccess-control-allow-methods: GET, POST, OPTIONS\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

struct SseSlot;

impl SseSlot {
    fn new() -> Option<Self> {
        let mut current = SSE_CLIENTS.load(Ordering::Relaxed);
        loop {
            if current >= MAX_SSE_CLIENTS {
                return None;
            }
            match SSE_CLIENTS.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self),
                Err(next) => current = next,
            }
        }
    }
}

impl Drop for SseSlot {
    fn drop(&mut self) {
        SSE_CLIENTS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn header_end(request: &[u8]) -> Option<(usize, usize)> {
    find_bytes(request, b"\r\n\r\n")
        .map(|offset| (offset, 4))
        .or_else(|| find_bytes(request, b"\n\n").map(|offset| (offset, 2)))
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

struct P2PManager {
    verbose: bool,
    table: Arc<ToolTable>,
    signal_url: String,
    room: String,
    identity_public: [u8; 32],
    identity_secret: [u8; 32],
    session_key: Option<[u8; 32]>,
    peer_identity: Option<String>,
}

impl P2PManager {
    fn new(verbose: bool, table: Arc<ToolTable>, signal_url: String, room: String) -> Self {
        let mut secret = [0_u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let public = x25519_public(secret);
        Self {
            verbose,
            table,
            signal_url,
            room,
            identity_public: public,
            identity_secret: secret,
            session_key: None,
            peer_identity: None,
        }
    }

    fn join_signal_room(&mut self) -> Result<(), TransportError> {
        let ws_url = signal_websocket_url(&self.signal_url, &self.room)?;
        if self.verbose {
            log_status(&format!("signaling websocket: {ws_url}"));
        }
        let mut request = ws_url.as_str().into_client_request()?;
        let mut nonce = [0_u8; 16];
        rand::rng().fill_bytes(&mut nonce);
        let key = base64::engine::general_purpose::STANDARD.encode(nonce);
        request.headers_mut().insert(
            "sec-websocket-key",
            key.parse().map_err(|_| TransportError::InvalidSignalUrl)?,
        );
        let (mut socket, _) = tungstenite::connect(request)?;
        let identity = hex::encode(self.identity_public);
        socket.send(Message::Text(
            json!({"type":"join","identity":identity})
                .to_string()
                .into(),
        ))?;
        if self.verbose {
            log_status("signaling join sent");
        }
        while let Ok(message) = socket.read() {
            match message {
                Message::Text(text) => self.handle_signal_message(&mut socket, &text)?,
                Message::Binary(bytes) => {
                    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                        self.handle_signal_message(&mut socket, &text)?;
                    }
                }
                Message::Ping(payload) => socket.send(Message::Pong(payload))?,
                Message::Close(_) => return Ok(()),
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_signal_message(
        &mut self,
        socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
        raw: &str,
    ) -> Result<(), TransportError> {
        if self.verbose {
            log_status(&format!("signaling recv: {raw}"));
        }
        let parsed: Value = serde_json::from_str(raw)?;
        let msg_type = parsed.get("type").and_then(Value::as_str).unwrap_or("");
        match msg_type {
            "joined" => {
                if let Some(peers) = parsed.get("peers").and_then(Value::as_array) {
                    for peer in peers.iter().filter_map(Value::as_str) {
                        self.send_offer(socket, peer)?;
                    }
                }
            }
            "peer_joined" => {
                if let Some(peer) = parsed.get("identity").and_then(Value::as_str) {
                    self.send_offer(socket, peer)?;
                }
            }
            "offer" => {
                if let Some(peer) = parsed.get("from").and_then(Value::as_str) {
                    self.establish_session(peer)?;
                    self.send_answer(socket, peer)?;
                }
            }
            "answer" => {
                if let Some(peer) = parsed.get("from").and_then(Value::as_str) {
                    self.establish_session(peer)?;
                }
            }
            "relay" => self.handle_relay(socket, &parsed)?,
            _ => {}
        }
        Ok(())
    }

    fn send_offer(
        &mut self,
        socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
        peer: &str,
    ) -> Result<(), TransportError> {
        self.peer_identity = Some(peer.to_string());
        let from = hex::encode(self.identity_public);
        socket.send(Message::Text(
            json!({"type":"offer","from":from,"to":peer,"data":{"type":"mcp_relay"}})
                .to_string()
                .into(),
        ))?;
        if self.verbose {
            log_status(&format!("sent offer to {peer}"));
        }
        Ok(())
    }

    fn send_answer(
        &mut self,
        socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
        peer: &str,
    ) -> Result<(), TransportError> {
        let from = hex::encode(self.identity_public);
        socket.send(Message::Text(json!({"type":"answer","from":from,"to":peer,"data":{"type":"mcp_relay","accepted":true}}).to_string().into()))?;
        if self.verbose {
            log_status(&format!("sent answer to {peer}"));
        }
        Ok(())
    }

    fn handle_relay(
        &mut self,
        socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
        parsed: &Value,
    ) -> Result<(), TransportError> {
        let from = parsed
            .get("from")
            .and_then(Value::as_str)
            .ok_or(TransportError::InvalidPeerIdentity)?;
        let data = parsed
            .get("data")
            .ok_or(TransportError::InvalidEncryptedPayload)?;
        self.ensure_relay_sender(from)?;
        if self.verbose {
            log_status(&format!("relay from {from}"));
        }
        let mcp_json = self.decrypt_relay_data(data)?;
        let msg: Value = serde_json::from_str(&mcp_json)?;
        if let Some(resp) = handle_message(self.verbose, &self.table, msg)? {
            let encrypted = self.encrypt_relay_payload(resp.as_bytes())?;
            let from_id = hex::encode(self.identity_public);
            socket.send(Message::Text(
                json!({"type":"relay","from":from_id,"to":from,"data":encrypted})
                    .to_string()
                    .into(),
            ))?;
        }
        Ok(())
    }

    fn establish_session(&mut self, peer: &str) -> Result<(), TransportError> {
        let peer_public = parse_identity(peer)?;
        let shared = x25519_shared(self.identity_secret, peer_public);
        let mut salt = [0_u8; 64];
        if self.identity_public < peer_public {
            salt[..32].copy_from_slice(&self.identity_public);
            salt[32..].copy_from_slice(&peer_public);
        } else {
            salt[..32].copy_from_slice(&peer_public);
            salt[32..].copy_from_slice(&self.identity_public);
        }
        let hk = Hkdf::<Sha256>::new(Some(&salt), &shared);
        let mut key = [0_u8; 32];
        hk.expand(RELAY_AD, &mut key)
            .map_err(|_| TransportError::Crypto)?;
        self.session_key = Some(key);
        self.peer_identity = Some(peer.to_string());
        Ok(())
    }

    fn encrypt_relay_payload(&self, plaintext: &[u8]) -> Result<Value, TransportError> {
        let key = self.session_key.ok_or(TransportError::HandshakeRequired)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        let mut nonce = [0_u8; 24];
        rand::rng().fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext)
            .map_err(|_| TransportError::Crypto)?;
        Ok(json!({
            "v": 1,
            "alg": "xchacha20poly1305",
            "nonce": hex::encode(nonce),
            "ciphertext": hex::encode(ciphertext)
        }))
    }

    fn decrypt_relay_data(&self, data: &Value) -> Result<String, TransportError> {
        let encrypted = if let Some(text) = data.as_str() {
            serde_json::from_str::<Value>(text)?
        } else {
            data.clone()
        };
        if encrypted.get("v").and_then(Value::as_i64) != Some(1) {
            return Err(TransportError::InvalidEncryptedPayload);
        }
        if encrypted.get("alg").and_then(Value::as_str) != Some("xchacha20poly1305") {
            return Err(TransportError::InvalidEncryptedPayload);
        }
        let nonce = hex::decode(
            encrypted
                .get("nonce")
                .and_then(Value::as_str)
                .ok_or(TransportError::InvalidEncryptedPayload)?,
        )
        .map_err(|_| TransportError::InvalidEncryptedPayload)?;
        let ciphertext = hex::decode(
            encrypted
                .get("ciphertext")
                .and_then(Value::as_str)
                .ok_or(TransportError::InvalidEncryptedPayload)?,
        )
        .map_err(|_| TransportError::InvalidEncryptedPayload)?;
        if nonce.len() != 24 {
            return Err(TransportError::InvalidEncryptedPayload);
        }
        let key = self.session_key.ok_or(TransportError::HandshakeRequired)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        let plaintext = cipher
            .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| TransportError::Crypto)?;
        String::from_utf8(plaintext).map_err(|_| TransportError::InvalidEncryptedPayload)
    }

    fn ensure_relay_sender(&self, from: &str) -> Result<(), TransportError> {
        match self.peer_identity.as_deref() {
            Some(peer) if peer == from => Ok(()),
            _ => Err(TransportError::InvalidPeerIdentity),
        }
    }
}

fn log_status(message: &str) {
    let now = terminal_time();
    eprintln!("[{now}] {message}");
}

fn terminal_time() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() % 86_400)
        .unwrap_or(0);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}

fn http_bind_addr(port: u16) -> (&'static str, u16) {
    ("127.0.0.1", port)
}

fn signal_websocket_url(raw_url: &str, room: &str) -> Result<Url, TransportError> {
    let base = if raw_url.starts_with("ws://")
        || raw_url.starts_with("wss://")
        || raw_url.starts_with("http://")
        || raw_url.starts_with("https://")
    {
        raw_url.to_string()
    } else {
        format!("https://{raw_url}")
    };
    let Some((scheme, rest)) = base.split_once("://") else {
        return Err(TransportError::InvalidSignalUrl);
    };
    let ws_scheme = match scheme {
        "http" => "ws",
        "https" => "wss",
        "ws" | "wss" => scheme,
        _ => return Err(TransportError::InvalidSignalUrl),
    };
    let rest = rest.trim_end_matches('/');
    Url::parse(&format!("{ws_scheme}://{rest}/signal/{room}")).map_err(Into::into)
}

fn parse_identity(hex_value: &str) -> Result<[u8; 32], TransportError> {
    let bytes = hex::decode(hex_value).map_err(|_| TransportError::InvalidPeerIdentity)?;
    bytes
        .try_into()
        .map_err(|_| TransportError::InvalidPeerIdentity)
}

fn x25519_public(secret: [u8; 32]) -> [u8; 32] {
    use x25519_dalek::{PublicKey, StaticSecret};
    let secret = StaticSecret::from(secret);
    PublicKey::from(&secret).to_bytes()
}

fn x25519_shared(secret: [u8; 32], public: [u8; 32]) -> [u8; 32] {
    use x25519_dalek::{PublicKey, StaticSecret};
    let secret = StaticSecret::from(secret);
    let public = PublicKey::from(public);
    secret.diffie_hellman(&public).to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_should_match_legacy_shape() {
        let url = signal_websocket_url("https://folkaround.undivisible.dev", "room").unwrap();
        assert_eq!(url.as_str(), "wss://folkaround.undivisible.dev/signal/room");
    }

    #[test]
    fn http_bind_addr_should_be_loopback_only() {
        assert_eq!(http_bind_addr(8080), ("127.0.0.1", 8080));
    }

    #[test]
    fn relay_sender_should_match_established_peer() {
        let table = Arc::new(ToolTable::new(folk_core::AccessMode::Full));
        let mut manager =
            P2PManager::new(false, table, "https://example.com".into(), "room".into());
        let peer = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        manager.establish_session(peer).unwrap();

        assert!(manager.ensure_relay_sender(peer).is_ok());
        assert!(
            manager
                .ensure_relay_sender(
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                )
                .is_err()
        );
    }

    #[test]
    fn stdio_reader_should_accept_raw_json_line() {
        let mut reader = BufReader::new(br#"{"jsonrpc":"2.0"}"#.as_slice());
        let body = read_stdio_message(&mut reader).unwrap().unwrap();
        assert_eq!(body, br#"{"jsonrpc":"2.0"}"#);
    }

    #[test]
    fn http_should_reject_oversized_body_before_reading_it() {
        let table = Arc::new(ToolTable::new(folk_core::AccessMode::Full));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_http_client(stream, false, table).unwrap();
        });

        let mut client = TcpStream::connect(addr).unwrap();
        write!(
            client,
            "POST /message HTTP/1.1\r\nhost: localhost\r\ncontent-length: {}\r\n\r\n",
            MAX_HTTP_BODY_BYTES + 1
        )
        .unwrap();
        client.flush().unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();

        assert!(response.starts_with("HTTP/1.1 413 Payload Too Large"));
    }

    #[test]
    fn sse_slot_should_enforce_connection_limit() {
        let slots = (0..MAX_SSE_CLIENTS)
            .map(|_| SseSlot::new().unwrap())
            .collect::<Vec<_>>();
        assert!(SseSlot::new().is_none());
        drop(slots);
        assert!(SseSlot::new().is_some());
    }
}
