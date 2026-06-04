use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
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
                    eprintln!("[folk] json error: {err}");
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
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    if verbose {
        eprintln!("[folk] HTTP listening on http://127.0.0.1:{port}/");
    }
    for stream in listener.incoming() {
        let table = Arc::clone(&table);
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    let _ = handle_http_client(stream, verbose, table);
                });
            }
            Err(err) if verbose => eprintln!("[folk] HTTP accept error: {err}"),
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
                eprintln!("[folk] signaling error: {err}");
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
            expected_len = Some(offset + len + content_length(&buffer[..offset]));
        }
        if let Some(len) = expected_len {
            if buffer.len() >= len {
                break;
            }
        } else if buffer.len() > 64 * 1024 {
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
            eprintln!("[folk] signaling websocket: {ws_url}");
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
            eprintln!("[folk] signaling join sent");
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
            eprintln!("[folk] signaling recv: {raw}");
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
            eprintln!("[folk] sent offer to {peer}");
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
            eprintln!("[folk] sent answer to {peer}");
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
        if self.verbose {
            eprintln!("[folk] relay from {from}");
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
    fn stdio_reader_should_accept_raw_json_line() {
        let mut reader = BufReader::new(br#"{"jsonrpc":"2.0"}"#.as_slice());
        let body = read_stdio_message(&mut reader).unwrap().unwrap();
        assert_eq!(body, br#"{"jsonrpc":"2.0"}"#);
    }
}
