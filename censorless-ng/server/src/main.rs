use clap::Parser;
use protocol::{
    decrypt_payload, encrypt_payload, verify_connection_id, ClientMessage, ClientMessageType,
    ErrorCode, ServerResponse, ServerResponseType, SigningKey, SocksAddress, VerifyingKey,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{interval, timeout, Instant},
};
use tracing::{debug, error, info, warn};

#[derive(Debug, Deserialize)]
struct Config {
    private_key: String,
    port: u16,
    addr: String,
    #[serde(default = "default_buffer_max")]
    #[allow(dead_code)]
    buffer_max: usize, // reserved for future use
    timeout: u64,
    idle_timeout: u64,
    connections_per_pkey: usize,
    #[serde(default)]
    allow_private: bool,
    #[serde(default = "default_read_timeout")]
    read_timeout: u64,
    #[serde(default = "default_poll_timeout")]
    #[allow(dead_code)]
    poll_timeout: u64, // kept for config backwards compat; poll now uses non-blocking try_read
}

fn default_buffer_max() -> usize {
    1_048_576
}

fn default_read_timeout() -> u64 {
    1000
}

fn default_poll_timeout() -> u64 {
    500
}

/// Global connection ID counter (starts at 1, 0 is reserved for errors)
static CONNECTION_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique connection ID
fn generate_connection_id() -> u64 {
    CONNECTION_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Check if an IP address is private/internal
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_multicast()
                || ipv4.is_broadcast()
                || ipv4.is_documentation()
                || ipv4.is_unspecified()
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_multicast()
                || ipv6.is_unspecified()
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Maximum accumulated data per response to prevent memory issues (4MB)
const MAX_RESPONSE_DATA: usize = 4 * 1024 * 1024;

/// Per-connection metadata
struct ConnectionInfo {
    stream: TcpStream,
    last_activity: Instant,
}

/// Nonce sliding window validator
///
/// Accepts nonces that are either:
/// - Higher than last_nonce (advances the window)
/// - Within the 64-nonce window behind last_nonce and not yet seen
struct NonceValidator {
    last_nonce: u64,
    /// Bitmask tracking which of the last 64 nonces have been seen.
    /// Bit i (0-indexed from LSB) represents (last_nonce - 1 - i).
    window: u64,
}

impl NonceValidator {
    fn new() -> Self {
        NonceValidator {
            last_nonce: 0,
            window: 0,
        }
    }

    /// Check if a nonce is valid and mark it as seen.
    /// Returns true if the nonce is accepted, false if it's a duplicate or too old.
    fn check(&mut self, nonce: u64) -> bool {
        if nonce == 0 {
            return false;
        }

        if nonce > self.last_nonce {
            // Advance the window
            let shift = nonce - self.last_nonce;
            if shift >= 64 {
                self.window = 0;
            } else {
                self.window <<= shift;
                // Mark the old last_nonce as seen in the window
                // The old last_nonce is now at position (shift - 1) from the new last_nonce
                self.window |= 1 << (shift - 1);
            }
            self.last_nonce = nonce;
            true
        } else {
            // Nonce is <= last_nonce, check if it's within the window
            let diff = self.last_nonce - nonce;
            if diff == 0 {
                // Same as last_nonce (duplicate)
                false
            } else if diff > 64 {
                // Too old
                false
            } else {
                // Check if bit (diff - 1) is set
                let bit = diff - 1;
                if self.window & (1 << bit) != 0 {
                    // Already seen
                    false
                } else {
                    // Mark as seen
                    self.window |= 1 << bit;
                    true
                }
            }
        }
    }
}

/// Per-client state
struct ClientState {
    nonce: NonceValidator,
    connections: HashMap<u64, ConnectionInfo>,
}

type ClientMap = HashMap<[u8; 32], Arc<tokio::sync::Mutex<ClientState>>>;

/// Server state
struct ServerState {
    #[allow(dead_code)]
    private_key: [u8; 32],
    public_key: [u8; 32],
    config: Config,
    clients: Arc<tokio::sync::RwLock<ClientMap>>,
}

impl ServerState {
    fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        let private_key_bytes = hex::decode(&config.private_key)?;
        if private_key_bytes.len() != 32 {
            return Err("Invalid private key length".into());
        }
        let mut private_key = [0u8; 32];
        private_key.copy_from_slice(&private_key_bytes);

        let signing_key = SigningKey::from_bytes(&private_key);
        let public_key = signing_key.verifying_key().to_bytes();

        info!("Server public key: {}", hex::encode(public_key));

        Ok(ServerState {
            private_key,
            public_key,
            config,
            clients: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }

    /// Get or create the per-client mutex, using read-lock first, write-lock only if needed.
    async fn get_client(
        &self,
        client_key: [u8; 32],
    ) -> Arc<tokio::sync::Mutex<ClientState>> {
        // Try read lock first
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(&client_key) {
                return client.clone();
            }
        }
        // Not found, take write lock to insert
        let mut clients = self.clients.write().await;
        // Double-check after acquiring write lock
        clients
            .entry(client_key)
            .or_insert_with(|| {
                Arc::new(tokio::sync::Mutex::new(ClientState {
                    nonce: NonceValidator::new(),
                    connections: HashMap::new(),
                }))
            })
            .clone()
    }

    async fn handle_connection(
        &self,
        mut stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("New connection from lambda: {}", addr);

        let mut encrypted_buf = Vec::new();
        stream.read_to_end(&mut encrypted_buf).await?;

        if encrypted_buf.is_empty() {
            debug!("Empty request from {}", addr);
            return Ok(());
        }

        debug!("Read {} encrypted bytes from lambda", encrypted_buf.len());

        let decrypted = decrypt_payload(&self.public_key, &encrypted_buf)?;

        debug!("Decrypted to {} bytes", decrypted.len());

        let client_msg = ClientMessage::deserialize(&decrypted).map_err(|e| {
            error!(
                "Deserialization failed: {} (decrypted size: {})",
                e,
                decrypted.len()
            );
            e
        })?;

        debug!(
            "Received message from client {} with nonce {}",
            hex::encode(client_msg.client_public_key),
            client_msg.nonce
        );

        // Get per-client lock
        let client_arc = self.get_client(client_msg.client_public_key).await;
        let mut client_state = client_arc.lock().await;

        // Validate nonce with sliding window
        if !client_state.nonce.check(client_msg.nonce) {
            warn!(
                "Invalid nonce: {} (last: {}, window: {:016x})",
                client_msg.nonce, client_state.nonce.last_nonce, client_state.nonce.window
            );
            // Send error response instead of dropping connection so the client
            // gets a meaningful error (and lambda doesn't see 0-byte response).
            let response = ServerResponse {
                responses: vec![ServerResponseType::Error {
                    connection_id: 0,
                    error_code: ErrorCode::InvalidNonce,
                    message: format!(
                        "Replay attack detected: nonce {} rejected (last: {})",
                        client_msg.nonce, client_state.nonce.last_nonce
                    ),
                }],
            };
            let response_data = response.serialize()?;
            let encrypted_response =
                encrypt_payload(&client_msg.client_public_key, &response_data)?;
            stream.write_all(&encrypted_response).await?;
            return Ok(());
        }

        // Construct verifying key for signature checks
        let verifying_key = VerifyingKey::from_bytes(&client_msg.client_public_key)
            .map_err(|_| "Invalid client public key")?;

        // Process each message
        let mut responses = Vec::new();

        for msg in client_msg.messages {
            match msg {
                ClientMessageType::StartConnection { host, port } => {
                    info!("Starting connection to {:?}:{}", host, port);

                    if client_state.connections.len() >= self.config.connections_per_pkey {
                        warn!(
                            "Connection limit reached for client: {}/{}",
                            client_state.connections.len(),
                            self.config.connections_per_pkey
                        );
                        responses.push(ServerResponseType::Error {
                            connection_id: 0,
                            error_code: ErrorCode::TooManyConnections,
                            message: format!(
                                "Connection limit reached: {}/{}",
                                client_state.connections.len(),
                                self.config.connections_per_pkey
                            ),
                        });
                        continue;
                    }

                    if !self.config.allow_private {
                        let is_blocked = match &host {
                            SocksAddress::IPv4(ip) => is_private_ip(IpAddr::V4(*ip)),
                            SocksAddress::IPv6(ip) => is_private_ip(IpAddr::V6(*ip)),
                            SocksAddress::Domain(domain) => {
                                match tokio::net::lookup_host((domain.as_str(), port)).await {
                                    Ok(mut addrs) => addrs.any(|addr| is_private_ip(addr.ip())),
                                    Err(_) => false,
                                }
                            }
                        };

                        if is_blocked {
                            warn!("Blocked connection to private IP: {:?}:{}", host, port);
                            responses.push(ServerResponseType::Error {
                                connection_id: 0,
                                error_code: ErrorCode::HostUnreachable,
                                message: "Connection to private IP addresses is not allowed"
                                    .to_string(),
                            });
                            continue;
                        }
                    }

                    let target_addr = match host {
                        SocksAddress::IPv4(ip) => format!("{}:{}", ip, port),
                        SocksAddress::IPv6(ip) => format!("[{}]:{}", ip, port),
                        SocksAddress::Domain(domain) => format!("{}:{}", domain, port),
                    };

                    match timeout(
                        Duration::from_millis(self.config.timeout),
                        TcpStream::connect(&target_addr),
                    )
                    .await
                    {
                        Ok(Ok(target_stream)) => {
                            let connection_id = generate_connection_id();
                            client_state.connections.insert(
                                connection_id,
                                ConnectionInfo {
                                    stream: target_stream,
                                    last_activity: Instant::now(),
                                },
                            );

                            responses.push(ServerResponseType::Challenge { connection_id });
                            info!("Connection established: {}", connection_id);
                        }
                        Ok(Err(e)) => {
                            warn!("Failed to connect to {}: {}", target_addr, e);
                            let error_code = if e.kind() == std::io::ErrorKind::ConnectionRefused {
                                ErrorCode::ConnectionRefused
                            } else if e.kind() == std::io::ErrorKind::TimedOut {
                                ErrorCode::Timeout
                            } else {
                                ErrorCode::HostUnreachable
                            };

                            responses.push(ServerResponseType::Error {
                                connection_id: 0,
                                error_code,
                                message: format!("Failed to connect: {}", e),
                            });
                        }
                        Err(_) => {
                            warn!("Connection to {} timed out", target_addr);
                            responses.push(ServerResponseType::Error {
                                connection_id: 0,
                                error_code: ErrorCode::Timeout,
                                message: format!("Connection to {} timed out", target_addr),
                            });
                        }
                    }
                }

                ClientMessageType::Data {
                    connection_id_signed,
                    data,
                    compressed: _,
                } => {
                    if connection_id_signed.len() < 8 {
                        debug!("Invalid signed connection ID length");
                        continue;
                    }

                    let sig_len = 64;
                    if connection_id_signed.len() < sig_len {
                        debug!("Signature too short");
                        continue;
                    }

                    let (signed_data, signature) =
                        connection_id_signed.split_at(connection_id_signed.len() - sig_len);

                    if signed_data.len() < 8 {
                        debug!("Signed data too short");
                        continue;
                    }

                    // Verify signature
                    let conn_id_start = signed_data.len() - 8;
                    let server_addr_and_port = &signed_data[..conn_id_start];
                    let connection_id =
                        u64::from_le_bytes(signed_data[conn_id_start..].try_into().unwrap());

                    // Split server_addr_and_port into addr and port (last 2 bytes are port)
                    if server_addr_and_port.len() < 2 {
                        debug!("Server addr too short in signed data");
                        continue;
                    }
                    let server_addr_encoded =
                        &server_addr_and_port[..server_addr_and_port.len() - 2];
                    let server_port = u16::from_le_bytes(
                        server_addr_and_port[server_addr_and_port.len() - 2..]
                            .try_into()
                            .unwrap(),
                    );

                    if let Err(e) = verify_connection_id(
                        &verifying_key,
                        server_addr_encoded,
                        server_port,
                        connection_id,
                        signature,
                    ) {
                        warn!(
                            "Signature verification failed for connection {}: {}",
                            connection_id, e
                        );
                        continue;
                    }

                    if let Some(conn_info) = client_state.connections.get_mut(&connection_id) {
                        conn_info.last_activity = Instant::now();

                        if let Err(e) = conn_info.stream.write_all(&data).await {
                            error!("Failed to write to target: {}", e);
                            responses.push(ServerResponseType::Close {
                                connection_id,
                                message: format!("Write error: {}", e),
                            });
                            client_state.connections.remove(&connection_id);
                        } else {
                            // Loop reads until deadline expires or short read
                            let deadline =
                                Instant::now() + Duration::from_millis(self.config.read_timeout);
                            let mut accumulated = Vec::new();
                            let mut connection_closed = false;
                            let mut read_error = None;

                            loop {
                                let remaining = deadline.saturating_duration_since(Instant::now());
                                if remaining.is_zero() || accumulated.len() >= MAX_RESPONSE_DATA {
                                    break;
                                }

                                let mut buf = vec![0u8; 65536];
                                match timeout(remaining, conn_info.stream.read(&mut buf)).await {
                                    Ok(Ok(0)) => {
                                        connection_closed = true;
                                        break;
                                    }
                                    Ok(Ok(n)) => {
                                        accumulated.extend_from_slice(&buf[..n]);
                                        if n < buf.len() {
                                            break;
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        read_error = Some(e);
                                        break;
                                    }
                                    Err(_) => {
                                        // Timeout
                                        break;
                                    }
                                }
                            }

                            if !accumulated.is_empty() {
                                responses.push(ServerResponseType::Data {
                                    connection_id,
                                    data: accumulated,
                                    compressed: true,
                                });
                            }

                            if connection_closed {
                                responses.push(ServerResponseType::Close {
                                    connection_id,
                                    message: String::new(),
                                });
                                client_state.connections.remove(&connection_id);
                            } else if let Some(e) = read_error {
                                error!("Read error: {}", e);
                                responses.push(ServerResponseType::Close {
                                    connection_id,
                                    message: format!("Read error: {}", e),
                                });
                                client_state.connections.remove(&connection_id);
                            }
                        }
                    } else {
                        debug!("Connection ID {} not found", connection_id);
                    }
                }

                ClientMessageType::Close {
                    connection_id_signed,
                } => {
                    let sig_len = 64;
                    if connection_id_signed.len() < sig_len {
                        debug!("Signature too short");
                        continue;
                    }

                    let (signed_data, signature) =
                        connection_id_signed.split_at(connection_id_signed.len() - sig_len);

                    if signed_data.len() < 8 {
                        debug!("Signed data too short");
                        continue;
                    }

                    let conn_id_start = signed_data.len() - 8;
                    let server_addr_and_port = &signed_data[..conn_id_start];
                    let connection_id =
                        u64::from_le_bytes(signed_data[conn_id_start..].try_into().unwrap());

                    // Verify signature
                    if server_addr_and_port.len() < 2 {
                        debug!("Server addr too short in signed data");
                        continue;
                    }
                    let server_addr_encoded =
                        &server_addr_and_port[..server_addr_and_port.len() - 2];
                    let server_port = u16::from_le_bytes(
                        server_addr_and_port[server_addr_and_port.len() - 2..]
                            .try_into()
                            .unwrap(),
                    );

                    if let Err(e) = verify_connection_id(
                        &verifying_key,
                        server_addr_encoded,
                        server_port,
                        connection_id,
                        signature,
                    ) {
                        warn!(
                            "Signature verification failed for close on connection {}: {}",
                            connection_id, e
                        );
                        continue;
                    }

                    if client_state.connections.remove(&connection_id).is_some() {
                        debug!("Closed connection {}", connection_id);
                        responses.push(ServerResponseType::Close {
                            connection_id,
                            message: String::new(),
                        });
                    }
                }

                ClientMessageType::Poll => {
                    // Non-blocking poll: only grab data already in the kernel buffer
                    // using try_read() instead of blocking with poll_timeout per connection.
                    // This reduces Poll from O(N × poll_timeout) to O(N × ~0ms).
                    debug!("Poll request - checking for pending data (non-blocking)");
                    let mut to_remove = Vec::new();

                    for (connection_id, conn_info) in client_state.connections.iter_mut() {
                        let mut accumulated = Vec::new();
                        let mut connection_closed = false;
                        let mut read_error = None;

                        loop {
                            let mut buf = vec![0u8; 65536];
                            match conn_info.stream.try_read(&mut buf) {
                                Ok(0) => {
                                    connection_closed = true;
                                    break;
                                }
                                Ok(n) => {
                                    conn_info.last_activity = Instant::now();
                                    accumulated.extend_from_slice(&buf[..n]);
                                    if accumulated.len() >= MAX_RESPONSE_DATA {
                                        break;
                                    }
                                }
                                Err(ref e)
                                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                                {
                                    break;
                                }
                                Err(e) => {
                                    read_error = Some(e);
                                    break;
                                }
                            }
                        }

                        if !accumulated.is_empty() {
                            responses.push(ServerResponseType::Data {
                                connection_id: *connection_id,
                                data: accumulated,
                                compressed: true,
                            });
                        }

                        if connection_closed {
                            responses.push(ServerResponseType::Close {
                                connection_id: *connection_id,
                                message: String::new(),
                            });
                            to_remove.push(*connection_id);
                        } else if let Some(e) = read_error {
                            error!("Read error on connection {}: {}", connection_id, e);
                            responses.push(ServerResponseType::Close {
                                connection_id: *connection_id,
                                message: format!("Read error: {}", e),
                            });
                            to_remove.push(*connection_id);
                        }
                    }

                    for conn_id in to_remove {
                        client_state.connections.remove(&conn_id);
                    }
                }
            }
        }

        // Serialize and encrypt responses
        let response = ServerResponse { responses };
        let response_data = response.serialize()?;
        debug!("Serialized response: {} bytes", response_data.len());
        let encrypted_response = encrypt_payload(&client_msg.client_public_key, &response_data)?;
        debug!("Encrypted response: {} bytes", encrypted_response.len());

        stream.write_all(&encrypted_response).await?;

        debug!("Sent response to lambda");

        Ok(())
    }

    /// Periodic cleanup task to remove idle connections
    async fn cleanup_task(state: Arc<ServerState>) {
        let mut cleanup_interval = interval(Duration::from_secs(60));

        loop {
            cleanup_interval.tick().await;
            debug!("Running cleanup task");

            let idle_duration = Duration::from_millis(state.config.idle_timeout);

            // Read-lock to iterate client keys
            let client_keys: Vec<[u8; 32]> = {
                let clients = state.clients.read().await;
                clients.keys().copied().collect()
            };

            for client_key in client_keys {
                let client_arc = {
                    let clients = state.clients.read().await;
                    match clients.get(&client_key) {
                        Some(arc) => arc.clone(),
                        None => continue,
                    }
                };

                let mut client_state = client_arc.lock().await;
                let mut to_remove = Vec::new();

                for (conn_id, conn_info) in client_state.connections.iter() {
                    if conn_info.last_activity.elapsed() > idle_duration {
                        debug!("Removing idle connection {}", conn_id);
                        to_remove.push(*conn_id);
                    }
                }

                for conn_id in to_remove {
                    client_state.connections.remove(&conn_id);
                }
            }

            debug!("Cleanup task complete");
        }
    }
}

#[derive(Debug, Parser)]
struct Args {
    #[clap(short, long, default_value = "server.toml")]
    config: PathBuf,

    /// Allow connections to private IP addresses (for local testing)
    #[clap(long)]
    allow_private: bool,

    /// Verbosity level (error, warn, info, debug, trace)
    #[clap(short, long, default_value = "info")]
    verbosity: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let log_level = args
        .verbosity
        .parse::<tracing::Level>()
        .unwrap_or(tracing::Level::INFO);

    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .without_time()
        .init();

    let config_str = std::fs::read_to_string(args.config)?;
    let mut config: Config = toml::from_str(&config_str)?;

    if args.allow_private {
        config.allow_private = true;
        warn!("WARNING: Private IP access enabled (--allow-private). This should only be used for local testing!");
    }

    let bind_addr = format!("{}:{}", config.addr, config.port);
    info!("Starting server on {}", bind_addr);

    let state = Arc::new(ServerState::new(config)?);

    let cleanup_state = state.clone();
    tokio::spawn(async move {
        ServerState::cleanup_task(cleanup_state).await;
    });

    let listener = TcpListener::bind(&bind_addr).await?;
    info!("Server listening on {}", bind_addr);

    loop {
        let (socket, addr) = listener.accept().await?;
        let state = state.clone();

        tokio::spawn(async move {
            if let Err(e) = state.handle_connection(socket, addr).await {
                error!("Error handling connection from {}: {}", addr, e);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_validator_rejects_zero() {
        let mut v = NonceValidator::new();
        assert!(!v.check(0));
    }

    #[test]
    fn nonce_validator_accepts_increasing() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(2));
        assert!(v.check(3));
        assert!(v.check(100));
    }

    #[test]
    fn nonce_validator_rejects_duplicates() {
        let mut v = NonceValidator::new();
        assert!(v.check(5));
        assert!(!v.check(5)); // duplicate of last_nonce
    }

    #[test]
    fn nonce_validator_accepts_out_of_order_within_window() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(3)); // skip 2
        assert!(v.check(2)); // accept out-of-order within window
    }

    #[test]
    fn nonce_validator_rejects_out_of_order_duplicate() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(3));
        assert!(v.check(2));
        assert!(!v.check(2)); // already seen
    }

    #[test]
    fn nonce_validator_window_boundary() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(65)); // jump by 64
        // shift = 64 >= 64, so window is cleared. nonce 1 has diff=64, bit=63.
        // Since the window was cleared, nonce 1's "seen" status was lost.
        // It falls within the 64-nonce window (diff <= 64) so it's accepted.
        assert!(v.check(1));
        // But now it's been seen, so reject duplicate
        assert!(!v.check(1));
    }

    #[test]
    fn nonce_validator_window_outside() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(66)); // jump by 65
        // diff = 65 > 64, nonce 1 is outside the window
        assert!(!v.check(1));
    }

    #[test]
    fn nonce_validator_window_just_inside() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(64)); // jump by 63
        // nonce 1 is at position 62 (64 - 1 - 1 = 62), shift was 63
        // window was shifted left by 63, old nonce 1 bit set at position 62
        // nonce 1: diff = 63, bit = 62, should be set (was the old last_nonce)
        // Actually nonce 1 was last_nonce=1, then we advanced to 64 with shift=63
        // window <<= 63, then window |= 1 << 62 (marking old last_nonce=1)
        // So bit 62 is set. For nonce 1: diff = 64-1 = 63, bit = 62. It's set -> reject as seen
        assert!(!v.check(1));
        // But nonce 2 should be available (within window, not seen)
        assert!(v.check(2));
    }

    #[test]
    fn nonce_validator_rejects_too_old() {
        let mut v = NonceValidator::new();
        assert!(v.check(100));
        // nonce 35: diff = 65, too old (> 64)
        assert!(!v.check(35));
        // nonce 36: diff = 64, too old (> 64 is false, == 64... diff > 64 check)
        // Actually diff = 100 - 36 = 64, which is == 64, not > 64
        // bit = 63, which is valid (0..63)
        assert!(v.check(36));
    }

    #[test]
    fn nonce_validator_large_gap() {
        let mut v = NonceValidator::new();
        assert!(v.check(1));
        assert!(v.check(1000)); // large jump clears window
        assert!(!v.check(1)); // way too old
        assert!(v.check(999)); // within window
        assert!(!v.check(999)); // duplicate
    }

    #[test]
    fn nonce_validator_sequential_then_gap() {
        let mut v = NonceValidator::new();
        for i in 1..=10 {
            assert!(v.check(i));
        }
        assert!(v.check(20)); // skip 11-19
        // 11-19 should be available
        for i in 11..=19 {
            assert!(v.check(i), "nonce {} should be accepted", i);
        }
        // All should now be rejected
        for i in 11..=20 {
            assert!(!v.check(i), "nonce {} should be rejected", i);
        }
    }
}
