//! Shared security primitives for the loopback JSON control transports.

use std::fs::OpenOptions;
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::AutomationError;

/// The request that authenticates a TCP connection: with the token itself (the original form),
/// or with the client's proof after [`CHALLENGE_METHOD`].
pub const AUTH_METHOD: &str = "auth";
/// The first frame of challenge authentication: it carries a client nonce, never the token (NV-6).
pub const CHALLENGE_METHOD: &str = "auth.challenge";
/// Maximum encoded JSON request line, including its newline.
pub const MAX_REQUEST_BYTES: usize = 1 << 20;
/// Maximum simultaneously serviced TCP connections per listener.
pub const MAX_CONNECTIONS: usize = 16;
/// Maximum commands or method calls in one batch.
pub const MAX_BATCH_STEPS: usize = 256;
/// Idle/read and write timeout for loopback TCP connections.
pub const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a new TCP connection has to authenticate, however active it is. [`IO_TIMEOUT`] starts
/// again with every read, so a client that kept sending without ever completing `auth` held one
/// of the [`MAX_CONNECTIONS`] permits for as long as it liked (#4965).
pub const AUTH_DEADLINE: Duration = Duration::from_secs(10);

const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// Result of reading one bounded JSON-lines frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRead {
    Eof,
    Line,
    TooLong,
}

/// Read one line without ever buffering more than [`MAX_REQUEST_BYTES`] plus one byte.
///
/// `TooLong` leaves the bytes read so far in `line` (lossily decoded); the rest of that line is
/// still unread unless `line` ends with a newline (see [`discard_rest_of_line`]). A line within
/// the limit that is not UTF-8 is an `InvalidData` error, and the whole line has been consumed.
pub fn read_bounded_line(reader: &mut impl BufRead, line: &mut String) -> std::io::Result<LineRead> {
    line.clear();
    let mut bytes = Vec::new();
    let n = std::io::Read::take(reader, (MAX_REQUEST_BYTES + 1) as u64).read_until(b'\n', &mut bytes)?;
    if n == 0 {
        Ok(LineRead::Eof)
    } else if n > MAX_REQUEST_BYTES {
        // A cut can split a multi-byte character; the length, not the text, decides.
        line.push_str(&String::from_utf8_lossy(&bytes));
        Ok(LineRead::TooLong)
    } else {
        *line = String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(LineRead::Line)
    }
}

/// Skip the rest of the current line (through its newline, or to EOF) without buffering it, so
/// a stream transport can answer an over-long request and keep reading the next one.
pub fn discard_rest_of_line(reader: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(());
        }
        if let Some(end) = buf.iter().position(|&b| b == b'\n') {
            reader.consume(end + 1);
            return Ok(());
        }
        let n = buf.len();
        reader.consume(n);
    }
}

/// Lowercase hexadecimal, two characters per byte.
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// 256 bits from the operating system CSPRNG, as 64 lowercase hexadecimal characters.
fn random_hex(what: &str) -> Result<String, AutomationError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| AutomationError::Other(format!("cannot generate {what}: {e}")))?;
    Ok(hex(&bytes))
}

/// Generate a 256-bit bearer token with the operating system CSPRNG.
pub fn generate_token() -> Result<String, AutomationError> {
    random_hex("control token")
}

/// Generate a fresh 256-bit nonce for challenge authentication ([`CHALLENGE_METHOD`]).
pub fn generate_nonce() -> Result<String, AutomationError> {
    random_hex("authentication nonce")
}

/// Whether `s` has the shape of a token or nonce: exactly 64 hexadecimal characters.
pub fn is_hex256(s: &str) -> bool {
    s.len() == TOKEN_HEX_LEN && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Accept only the fixed-width hexadecimal representation emitted by [`generate_token`].
pub fn validate_token(token: &str) -> Result<(), AutomationError> {
    if !is_hex256(token) {
        return Err(AutomationError::BadRequest("control token must contain exactly 64 hexadecimal characters".into()));
    }
    Ok(())
}

/// Compare fixed-width tokens without an early exit on a mismatching byte.
///
/// Hex letters match in either case: [`validate_token`] accepts uppercase, and the server keeps
/// the token lowercased, so a client sending the token exactly as it was configured must still
/// authenticate (#6425). Only `A`–`Z` fold, so a non-hex byte never matches a hex digit.
pub fn token_matches(expected: &str, supplied: &str) -> bool {
    if expected.len() != TOKEN_HEX_LEN || supplied.len() != TOKEN_HEX_LEN {
        return false;
    }
    expected.bytes().zip(supplied.bytes()).fold(0u8, |different, (a, b)| different | (a.to_ascii_lowercase() ^ b.to_ascii_lowercase())) == 0
}

/// Which side of the challenge handshake a proof comes from. Each side's proof is keyed by the
/// token over both nonces under its own label, so one side's proof never passes for the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofRole {
    Server,
    Client,
}

/// HMAC-SHA256, keyed by the (lowercased) control token, over the role's label and both nonces,
/// as 64 lowercase hexadecimal characters. `docs/control-protocol.md` gives the exact message.
pub fn handshake_proof(token: &str, role: ProofRole, client_nonce: &str, server_nonce: &str) -> Result<String, AutomationError> {
    let label: &[u8] = match role {
        ProofRole::Server => b"photocraft-control-v1 server",
        ProofRole::Client => b"photocraft-control-v1 client",
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(token.to_ascii_lowercase().as_bytes())
        .map_err(|e| AutomationError::Other(format!("cannot key the authentication proof: {e}")))?;
    for part in [label, b"\n", client_nonce.to_ascii_lowercase().as_bytes(), b"\n", server_nonce.to_ascii_lowercase().as_bytes()] {
        mac.update(part);
    }
    Ok(hex(&mac.finalize().into_bytes()))
}

/// Whether `supplied` is `role`'s proof for these nonces, compared without an early exit.
pub fn proof_matches(token: &str, role: ProofRole, client_nonce: &str, server_nonce: &str, supplied: &str) -> bool {
    handshake_proof(token, role, client_nonce, server_nonce).is_ok_and(|expected| token_matches(&expected, supplied))
}

/// Where a connection's authentication stands after one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStep {
    /// Authenticated: dispatch the frames that follow.
    Authenticated,
    /// A challenge was answered: the next frame must be `auth` with the client's proof.
    Challenged,
    /// Refused: send the reply, then close the connection.
    Rejected,
}

/// Server side of connection authentication, one per connection. It accepts either
///
/// - challenge authentication (NV-6): `auth.challenge {clientNonce}` is answered with
///   `{serverNonce, serverProof}`, then `auth {proof}` authenticates. The token never crosses the
///   wire, and the client learns that the server holds the token before revealing anything
///   derived from it; or
/// - the original one-frame `auth {token}`, kept so existing clients and scripts keep working.
///
/// No control method is answered before authentication.
#[derive(Debug, Default)]
pub struct ServerHandshake {
    /// The client and server nonces of the challenge this connection was given, until its `auth`.
    challenge: Option<(String, String)>,
}

impl ServerHandshake {
    /// A handshake for a freshly accepted connection: no challenge issued yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer one frame received before authentication. Never dispatches a control method.
    pub fn reply(&mut self, line: &str, token: &str) -> (Value, AuthStep) {
        let refused = |id: Value| (json!({"id": id, "ok": false, "error": "authentication required"}), AuthStep::Rejected);
        let authenticated = |id: Value| (json!({"id": id, "ok": true, "result": {"authenticated": true}}), AuthStep::Authenticated);
        let Ok(req) = serde_json::from_str::<Value>(line) else {
            return refused(Value::Null);
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let param = |key: &str| req.get("params").and_then(|p| p.get(key)).and_then(Value::as_str);
        match (req.get("method").and_then(Value::as_str), self.challenge.take()) {
            (Some(CHALLENGE_METHOD), None) => {
                let Some(client_nonce) = param("clientNonce").filter(|n| is_hex256(n)).map(str::to_ascii_lowercase) else {
                    return refused(id);
                };
                let Ok(server_nonce) = generate_nonce() else {
                    return (json!({"id": id, "ok": false, "error": "cannot generate an authentication challenge"}), AuthStep::Rejected);
                };
                let Ok(server_proof) = handshake_proof(token, ProofRole::Server, &client_nonce, &server_nonce) else {
                    return refused(id);
                };
                let reply = json!({"id": id, "ok": true, "result": {"serverNonce": server_nonce, "serverProof": server_proof}});
                self.challenge = Some((client_nonce, server_nonce));
                (reply, AuthStep::Challenged)
            }
            // After a challenge only the proof authenticates; the token is never needed.
            (Some(AUTH_METHOD), Some((client_nonce, server_nonce))) => match param("proof") {
                Some(proof) if proof_matches(token, ProofRole::Client, &client_nonce, &server_nonce, proof) => authenticated(id),
                _ => refused(id),
            },
            (Some(AUTH_METHOD), None) if token_matches(token, param("token").unwrap_or("")) => authenticated(id),
            _ => refused(id),
        }
    }
}

/// Answer a repeat `auth` or `auth.challenge` frame on an already authenticated connection.
/// Returns `None` for any other frame, which the caller dispatches as usual.
/// Without this, a second `auth` fell through to the method dispatcher as an unknown method.
pub fn repeat_auth_reply(line: &str) -> Option<Value> {
    let req: Value = serde_json::from_str(line).ok()?;
    if !matches!(req.get("method").and_then(Value::as_str), Some(AUTH_METHOD | CHALLENGE_METHOD)) {
        return None;
    }
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    Some(json!({"id": id, "ok": false, "error": "already authenticated"}))
}

fn read_token_file(path: &Path) -> Result<String, AutomationError> {
    // Validate what we actually opened, not just the name: an existing control-token file must be
    // a regular file that no other local principal can read. A freshly created one is 0600
    // (`create_token_file`); an operator-supplied one is checked here so a group- or world-readable
    // token is refused rather than trusted as the control credential (#4966).
    let mut file = std::fs::File::open(path).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
    let meta = file.metadata().map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(AutomationError::Io(format!("{}: control token file is not a regular file", path.display())));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 {
            return Err(AutomationError::Io(format!(
                "{}: control token file is readable by group or others (mode {:o}); restrict it to its owner (e.g. chmod 600)",
                path.display(),
                meta.mode() & 0o777
            )));
        }
    }
    let token = std::io::read_to_string(&mut file).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
    let token = token.trim().to_owned();
    validate_token(&token)?;
    Ok(token)
}

fn create_token_file(path: &Path, token: &str) -> Result<(), AutomationError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| AutomationError::Io(format!("{}: {e}", parent.display())))?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))?;
    writeln!(file, "{token}").map_err(|e| AutomationError::Io(format!("{}: {e}", path.display())))
}

/// Write a fresh token to `path`, replacing any earlier one (user-only on Unix), and return it.
/// The desktop app rotates the agent-connection token this way each time it starts listening, so
/// a token sent to a port another program took over after a crash stops working.
pub fn replace_token_file(path: &Path) -> Result<String, AutomationError> {
    let token = generate_token()?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(AutomationError::Io(format!("{}: {e}", path.display()))),
    }
    create_token_file(path, &token)?;
    Ok(token)
}

/// Resolve a server token. With no supplied token or file, a fresh token is returned.
/// A missing token file is created atomically; an existing one is read and validated.
pub fn server_token(supplied: Option<&str>, token_file: Option<&Path>) -> Result<String, AutomationError> {
    if supplied.is_some() && token_file.is_some() {
        return Err(AutomationError::BadRequest("use either a control token or a control token file, not both".into()));
    }
    if let Some(token) = supplied {
        validate_token(token)?;
        return Ok(token.to_ascii_lowercase());
    }
    let token = generate_token()?;
    let Some(path) = token_file else {
        return Ok(token);
    };
    match read_token_file(path) {
        Ok(existing) => Ok(existing.to_ascii_lowercase()),
        Err(AutomationError::Io(_)) if !path.exists() => {
            match create_token_file(path, &token) {
                Ok(()) => Ok(token),
                Err(AutomationError::Io(_)) if path.exists() => {
                    // Another process won the create-new race.
                    read_token_file(path).map(|v| v.to_ascii_lowercase())
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

/// Resolve the token used by a client. Clients never silently generate credentials.
pub fn client_token(supplied: Option<&str>, token_file: Option<&Path>) -> Result<String, AutomationError> {
    if supplied.is_some() && token_file.is_some() {
        return Err(AutomationError::BadRequest("use either a control token or a control token file, not both".into()));
    }
    if let Some(token) = supplied {
        validate_token(token)?;
        return Ok(token.to_ascii_lowercase());
    }
    token_file.map_or_else(
        || {
            Err(AutomationError::BadRequest(
                "bridge mode needs --control-token, --control-token-file, PHOTOCRAFT_CONTROL_TOKEN, or PHOTOCRAFT_CONTROL_TOKEN_FILE".into(),
            ))
        },
        |path| read_token_file(path).map(|v| v.to_ascii_lowercase()),
    )
}

/// Counts active connections and returns a permit only while below the configured maximum.
pub struct ConnectionLimiter {
    active: AtomicUsize,
    max: usize,
}

impl ConnectionLimiter {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self { active: AtomicUsize::new(0), max })
    }

    pub fn try_acquire(self: &Arc<Self>) -> Option<ConnectionPermit> {
        let mut current = self.active.load(Ordering::Acquire);
        loop {
            if current >= self.max {
                return None;
            }
            match self.active.compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(ConnectionPermit { limiter: Arc::clone(self) }),
                Err(actual) => current = actual,
            }
        }
    }
}

pub struct ConnectionPermit {
    limiter: Arc<ConnectionLimiter>,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.limiter.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Apply idle and write timeouts before handing a socket to a connection worker.
pub fn configure_stream(stream: &std::net::TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))
}

/// The read half of a control connection. Until [`Self::authenticated`] is called, reads fail
/// with `TimedOut` once the authentication deadline has passed, whatever arrived in the meantime;
/// the caller then drops the connection and its [`ConnectionPermit`].
pub struct AuthDeadlineStream {
    stream: std::net::TcpStream,
    /// `None` once authenticated.
    deadline: Option<Instant>,
}

impl AuthDeadlineStream {
    /// `stream` must authenticate within `limit` from now (normally [`AUTH_DEADLINE`]).
    pub fn new(stream: std::net::TcpStream, limit: Duration) -> Self {
        // A limit too large to represent expires at once rather than never.
        let now = Instant::now();
        Self { stream, deadline: Some(now.checked_add(limit).unwrap_or(now)) }
    }

    /// The connection authenticated: only the idle timeout applies from here on.
    pub fn authenticated(&mut self) -> std::io::Result<()> {
        self.deadline = None;
        self.stream.set_read_timeout(Some(IO_TIMEOUT))
    }
}

impl Read for AuthDeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "not authenticated in time"));
            }
            // Wait no longer than what is left, so a silent client is dropped at the deadline too.
            self.stream.set_read_timeout(Some(left.min(IO_TIMEOUT)))?;
        }
        self.stream.read(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame through a fresh handshake: the reply and whether it authenticated.
    fn authentication_reply(line: &str, token: &str) -> (Value, bool) {
        let (reply, step) = ServerHandshake::new().reply(line, token);
        (reply, step == AuthStep::Authenticated)
    }

    /// An `auth.challenge` frame carrying `client_nonce`.
    fn challenge_line(client_nonce: &str) -> String {
        json!({"id": 1, "method": CHALLENGE_METHOD, "params": {"clientNonce": client_nonce}}).to_string()
    }

    /// An `auth` frame carrying the client's challenge `proof`.
    fn proof_line(proof: &str) -> String {
        json!({"id": 2, "method": AUTH_METHOD, "params": {"proof": proof}}).to_string()
    }

    /// NV-6: a client proves it holds the token, and learns that the server does, without the
    /// token crossing the wire.
    #[test]
    fn challenge_authentication_proves_the_token_both_ways_without_sending_it() {
        let token = generate_token().unwrap();
        let client_nonce = generate_nonce().unwrap();
        let mut server = ServerHandshake::new();
        let (reply, step) = server.reply(&challenge_line(&client_nonce), &token);
        assert_eq!(step, AuthStep::Challenged, "{reply}");
        assert!(!reply.to_string().contains(&token), "the challenge reply must not carry the token");
        let server_nonce = reply["result"]["serverNonce"].as_str().unwrap();
        let server_proof = reply["result"]["serverProof"].as_str().unwrap();
        assert!(is_hex256(server_nonce) && server_nonce != client_nonce);
        assert!(proof_matches(&token, ProofRole::Server, &client_nonce, server_nonce, server_proof));
        // A server holding another token cannot produce that proof.
        let other = generate_token().unwrap();
        assert!(!proof_matches(&other, ProofRole::Server, &client_nonce, server_nonce, server_proof));
        // The server's proof is not the client's: replaying it back does not authenticate.
        assert!(!proof_matches(&token, ProofRole::Client, &client_nonce, server_nonce, server_proof));

        let proof = handshake_proof(&token, ProofRole::Client, &client_nonce, server_nonce).unwrap();
        assert_ne!(proof, token);
        let (reply, step) = server.reply(&proof_line(&proof), &token);
        assert_eq!(step, AuthStep::Authenticated, "{reply}");
        assert_eq!(reply["result"]["authenticated"], true);
    }

    /// NV-6: after a challenge, a wrong, missing or token-shaped proof, the token itself, a second
    /// challenge and a control method are all refused, and a malformed nonce gets no challenge.
    #[test]
    fn challenge_authentication_refuses_a_wrong_or_missing_proof() {
        let token = generate_token().unwrap();
        let client_nonce = generate_nonce().unwrap();
        let wrong = handshake_proof(&generate_token().unwrap(), ProofRole::Client, &client_nonce, &client_nonce).unwrap();
        let bad_proofs = [proof_line(&wrong), proof_line(&token), json!({"id": 2, "method": AUTH_METHOD, "params": {}}).to_string()];
        for line in bad_proofs {
            let mut server = ServerHandshake::new();
            assert_eq!(server.reply(&challenge_line(&client_nonce), &token).1, AuthStep::Challenged);
            let (reply, step) = server.reply(&line, &token);
            assert_eq!(step, AuthStep::Rejected, "{line}: {reply}");
            assert_eq!(reply["error"], "authentication required");
        }
        // After a challenge the token itself no longer authenticates, nor does a second challenge.
        for line in [auth_line(&token), challenge_line(&client_nonce), r#"{"id":3,"method":"methods"}"#.to_owned()] {
            let mut server = ServerHandshake::new();
            assert_eq!(server.reply(&challenge_line(&client_nonce), &token).1, AuthStep::Challenged);
            assert_eq!(server.reply(&line, &token).1, AuthStep::Rejected, "{line}");
        }
        // A malformed nonce is refused before any proof is computed.
        for nonce in ["", "short", &"g".repeat(64), &"a".repeat(65)] {
            let (reply, step) = ServerHandshake::new().reply(&challenge_line(nonce), &token);
            assert_eq!(step, AuthStep::Rejected, "{nonce}: {reply}");
            assert!(reply.get("result").is_none());
        }
    }

    #[test]
    fn replaced_token_files_hold_a_new_valid_token() {
        let dir = std::env::temp_dir().join(format!("photocraft-replace-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("agent.token");
        let first = replace_token_file(&path).unwrap();
        assert_eq!(client_token(None, Some(&path)).unwrap(), first);
        let second = replace_token_file(&path).unwrap();
        assert_ne!(first, second);
        assert_eq!(client_token(None, Some(&path)).unwrap(), second);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_tokens_are_valid_and_distinct() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        validate_token(&a).unwrap();
        assert_ne!(a, b);
        assert!(token_matches(&a, &a));
        assert!(!token_matches(&a, &b));
        assert!(!token_matches(&a, "short"));
    }

    #[test]
    fn bounded_reader_rejects_an_oversized_line() {
        let input = format!("{}\n", "x".repeat(MAX_REQUEST_BYTES + 1));
        let mut reader = std::io::Cursor::new(input);
        let mut line = String::new();
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::TooLong);
        assert_eq!(line.len(), MAX_REQUEST_BYTES + 1);
        discard_rest_of_line(&mut reader).unwrap();
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::Eof);
    }

    #[test]
    fn bounded_reader_reports_a_cut_multibyte_line_as_too_long_and_bad_utf8_as_invalid_data() {
        // The cut at the limit falls inside a two-byte character.
        let input = format!("xx{}\nnext\n", "é".repeat(MAX_REQUEST_BYTES / 2));
        let mut reader = std::io::Cursor::new(input);
        let mut line = String::new();
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::TooLong);
        discard_rest_of_line(&mut reader).unwrap();
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::Line);
        assert_eq!(line, "next\n");
        // Invalid UTF-8 consumes its whole line, so the next read starts on the next line.
        let mut reader = std::io::Cursor::new(b"\xff\xfe\nnext\n".to_vec());
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::Line);
        assert_eq!(line, "next\n");
    }

    #[test]
    fn authentication_does_not_dispatch_without_the_token() {
        let token = generate_token().unwrap();
        let (reply, authenticated) = authentication_reply(r#"{"id":1,"method":"methods","params":{}}"#, &token);
        assert!(!authenticated);
        assert_eq!(reply["error"], "authentication required");
        assert!(reply.get("result").is_none());

        let line = json!({"id": 2, "method": AUTH_METHOD, "params": {"token": token}}).to_string();
        let (reply, authenticated) = authentication_reply(&line, &token);
        assert!(authenticated);
        assert_eq!(reply["result"]["authenticated"], true);
    }

    fn auth_line(token: &str) -> String {
        json!({"id": 1, "method": AUTH_METHOD, "params": {"token": token}}).to_string()
    }

    /// #6425: `validate_token` accepts uppercase hex and the server keeps the token lowercased, so a
    /// client sending the token exactly as it was configured was refused.
    #[test]
    fn a_token_authenticates_in_any_letter_case() {
        let configured = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let mixed = "0123456789AbCdEf0123456789aBcDeF0123456789ABCDEF0123456789abcdef";
        let expected = server_token(Some(configured), None).unwrap();
        for supplied in [configured, &configured.to_ascii_lowercase(), mixed] {
            let (reply, authenticated) = authentication_reply(&auth_line(supplied), &expected);
            assert!(authenticated, "{supplied}: {reply}");
            assert_eq!(reply["result"]["authenticated"], true);
        }
        // The comparison is case-insensitive in both directions.
        assert!(token_matches(configured, &configured.to_ascii_lowercase()));

        // A different token, or a non-hex character that only folds to a hex one, still fails.
        let other = "1123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let (_, authenticated) = authentication_reply(&auth_line(other), &expected);
        assert!(!authenticated);
        let not_hex = "0123456789ABCDEG0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        assert!(!token_matches(&expected, not_hex));
    }

    /// #6425: the same for a token file whose contents are uppercase.
    #[test]
    fn an_uppercase_token_file_authenticates_its_exact_contents() {
        let path = std::env::temp_dir().join(format!("photocraft-control-token-upper-{}-{}.txt", std::process::id(), generate_token().unwrap()));
        let contents = generate_token().unwrap().to_ascii_uppercase();
        create_token_file(&path, &contents).unwrap();
        let expected = server_token(None, Some(&path));
        std::fs::remove_file(&path).unwrap();
        let (reply, authenticated) = authentication_reply(&auth_line(&contents), &expected.unwrap());
        assert!(authenticated, "{reply}");
    }

    /// A repeat `auth` or `auth.challenge` on an authenticated connection is answered "already
    /// authenticated"; any other frame is left to the dispatcher.
    #[test]
    fn repeat_auth_on_an_authenticated_connection_says_so() {
        let line = json!({"id": 3, "method": AUTH_METHOD, "params": {"token": "anything"}}).to_string();
        let reply = repeat_auth_reply(&line).unwrap();
        assert_eq!(reply["id"], 3);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], "already authenticated");
        let challenge = json!({"id": 5, "method": CHALLENGE_METHOD, "params": {"clientNonce": "anything"}}).to_string();
        assert_eq!(repeat_auth_reply(&challenge).unwrap()["error"], "already authenticated");
        assert!(repeat_auth_reply(r#"{"id":4,"method":"methods"}"#).is_none());
        assert!(repeat_auth_reply("{not json").is_none());
    }

    #[test]
    fn token_file_round_trips_between_server_and_client() {
        let path = std::env::temp_dir().join(format!("photocraft-control-token-{}-{}.txt", std::process::id(), generate_token().unwrap()));
        let server = server_token(None, Some(&path)).unwrap();
        let client = client_token(None, Some(&path)).unwrap();
        assert_eq!(server, client);
        assert!(token_matches(&server, &client));
        std::fs::remove_file(path).unwrap();
    }

    /// A connected loopback pair: the accepted (server) end and the client end.
    fn tcp_pair() -> (std::net::TcpStream, std::net::TcpStream) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        configure_stream(&server).unwrap();
        (server, client)
    }

    /// #4965: activity must not extend the time a connection may stay unauthenticated.
    #[test]
    fn an_active_connection_is_dropped_at_the_authentication_deadline() {
        let (server, mut client) = tcp_pair();
        // A byte every 20 ms and never a newline: each read succeeds long before the idle timeout.
        let trickle = std::thread::spawn(move || {
            for _ in 0..150 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let mut reader = std::io::BufReader::new(AuthDeadlineStream::new(server, Duration::from_millis(300)));
        let mut line = String::new();
        let started = Instant::now();
        let err = read_bounded_line(&mut reader, &mut line).unwrap_err();
        assert!(matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock), "{err}");
        assert!(started.elapsed() >= Duration::from_millis(250), "not before the deadline: {:?}", started.elapsed());
        assert!(started.elapsed() < Duration::from_secs(3), "the trickle did not extend it: {:?}", started.elapsed());
        // It stays expired.
        assert!(read_bounded_line(&mut reader, &mut line).is_err());
        drop(reader);
        trickle.join().unwrap();
    }

    #[test]
    fn a_silent_connection_is_dropped_at_the_authentication_deadline() {
        let (server, _client) = tcp_pair();
        let mut reader = std::io::BufReader::new(AuthDeadlineStream::new(server, Duration::from_millis(200)));
        let mut line = String::new();
        let started = Instant::now();
        assert!(read_bounded_line(&mut reader, &mut line).is_err());
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    }

    #[test]
    fn an_authenticated_connection_outlives_the_authentication_deadline() {
        let (server, mut client) = tcp_pair();
        // The test thread writes the first line before the deadline starts. The sender writes the
        // second line once the connection has authenticated and the deadline has passed.
        client.write_all(b"one\n").unwrap();
        let limit = Duration::from_secs(1);
        let mut reader = std::io::BufReader::new(AuthDeadlineStream::new(server, limit));
        let deadline = Instant::now() + limit;
        let (tx, rx) = std::sync::mpsc::channel();
        let sender = std::thread::spawn(move || {
            rx.recv().unwrap();
            std::thread::sleep((deadline + Duration::from_millis(300)).saturating_duration_since(Instant::now()));
            client.write_all(b"two\n").unwrap();
        });
        let mut line = String::new();
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::Line);
        assert_eq!(line, "one\n");
        reader.get_mut().authenticated().unwrap();
        tx.send(()).unwrap();
        // Sent 300 ms after the deadline: only the idle timeout applies now.
        assert_eq!(read_bounded_line(&mut reader, &mut line).unwrap(), LineRead::Line);
        assert_eq!(line, "two\n");
        sender.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_token_file_must_be_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("photocraft-token-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token.txt");
        let token = "0123456789abcdef".repeat(4); // 64 hex chars, passes validate_token
        std::fs::write(&path, &token).unwrap();

        // A group- or world-readable token is refused: the control credential must not be shared (#4966).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token_file(&path).is_err(), "a 0644 token file must be refused");

        // Owner-only is opened, read and validated.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_token_file(&path).unwrap(), token, "a 0600 token file must be read");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn connection_limiter_releases_capacity() {
        let limiter = ConnectionLimiter::new(1);
        let permit = limiter.try_acquire().unwrap();
        assert!(limiter.try_acquire().is_none());
        drop(permit);
        assert!(limiter.try_acquire().is_some());
    }
}
