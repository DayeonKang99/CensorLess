use clap::Parser;
use protocol::{
    decrypt_payload, encrypt_payload, ClientMessage, ClientMessageType, ErrorCode, ServerResponse,
    ServerResponseType, SigningKey, SocksAddress,
};
use serde::Deserialize;
use tracing::{debug, error, info, warn};
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
    sync::Mutex,
    time::{interval, timeout, Instant},
};

#[derive(Debug, Deserialize)]
struct Config {
    private_key: String,
    port: u16,
    addr: String,
    buffer_max: usize,
    timeout: u64,
    idle_timeout: u64,
    connections_per_pkey: usize,
    #[serde(default)]
    allow_private: bool,
    #[serde(default = "default_read_timeout")]
    read_timeout: u64, // milliseconds to wait for target response after writing data
    #[serde(default = "default_poll_timeout")]
    poll_timeout: u64, // milliseconds to wait for target data during poll
}

fn default_read_timeout() -> u64 {
    1000 // 1 second - reasonable for most internet services
}

fn default_poll_timeout() -> u64 {
    500 // 500ms - aggressive but still practical
}

/// Global connection ID counter (starts at 1, 0 is reserved for errors)
static CONNECTION_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique connection ID
fn generate_connection_id() -> u64 {
    CONNECTION_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Check if an IP address is private/internal
/// Blocks: loopback, private ranges, link-local, multicast
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            // Loopback: 127.0.0.0/8
            ipv4.is_loopback()
                // Private: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || ipv4.is_private()
                // Link-local: 169.254.0.0/16
                || ipv4.is_link_local()
                // Multicast: 224.0.0.0/4
                || ipv4.is_multicast()
                // Broadcast: 255.255.255.255
                || ipv4.is_broadcast()
                // Documentation: 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
                || ipv4.is_documentation()
                // Unspecified: 0.0.0.0
                || ipv4.is_unspecified()
        }
        IpAddr::V6(ipv6) => {
            // Loopback: ::1
            ipv6.is_loopback()
                // Multicast: ff00::/8
                || ipv6.is_multicast()
                // Unspecified: ::
                || ipv6.is_unspecified()
                // Unique local: fc00::/7
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local: fe80::/10
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Per-connection metadata
struct ConnectionInfo {
    stream: TcpStream,
    last_activity: Instant,
    buffer_used: usize,
}

/// Per-client state
struct ClientState {
    last_nonce: u64,
    connections: HashMap<u64, ConnectionInfo>,
}

/// Server state
struct ServerState {
    private_key: [u8; 32],
    public_key: [u8; 32],
    config: Config,
    clients: Arc<Mutex<HashMap<[u8; 32], ClientState>>>,
}

impl ServerState {
    fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        // Parse private key from hex
        let private_key_bytes = hex::decode(&config.private_key)?;
        if private_key_bytes.len() != 32 {
            return Err("Invalid private key length".into());
        }
        let mut private_key = [0u8; 32];
        private_key.copy_from_slice(&private_key_bytes);

        // Derive public key from private key
        let signing_key = SigningKey::from_bytes(&private_key);
        let public_key = signing_key.verifying_key().to_bytes();

        info!("Server public key: {}", hex::encode(&public_key));

        Ok(ServerState {
            private_key,
            public_key,
            config,
            clients: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn handle_connection(
        &self,
        mut stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("New connection from lambda: {}", addr);

        // Read the entire encrypted payload from the lambda
        let mut encrypted_buf = Vec::new();
        stream.read_to_end(&mut encrypted_buf).await?;

        if encrypted_buf.is_empty() {
            debug!("Empty request from {}", addr);
            return Ok(());
        }

        debug!("Read {} encrypted bytes from lambda", encrypted_buf.len());

        // Decrypt the payload (using public key to match what client encrypted with)
        let decrypted = decrypt_payload(&self.public_key, &encrypted_buf)?;

        debug!("Decrypted to {} bytes", decrypted.len());

        // Deserialize the client message
        let client_msg = ClientMessage::deserialize(&decrypted).map_err(|e| {
            error!("Deserialization failed: {} (decrypted size: {})", e, decrypted.len());
            e
        })?;

        debug!(
            "Received message from client {} with nonce {}",
            hex::encode(&client_msg.client_public_key),
            client_msg.nonce
        );
        debug!("Will encrypt response with client public key: {}", hex::encode(&client_msg.client_public_key));

        // Get or create client state
        let mut clients = self.clients.lock().await;
        let client_state = clients
            .entry(client_msg.client_public_key)
            .or_insert(ClientState {
                last_nonce: 0,
                connections: HashMap::new(),
            });

        // Validate nonce (must be strictly increasing)
        if client_msg.nonce <= client_state.last_nonce {
            warn!(
                "Invalid nonce: {} <= {}",
                client_msg.nonce, client_state.last_nonce
            );
            return Err("Replay attack detected: nonce must be strictly increasing".into());
        }
        client_state.last_nonce = client_msg.nonce;

        // Process each message
        let mut responses = Vec::new();

        for msg in client_msg.messages {
            match msg {
                ClientMessageType::StartConnection { host, port } => {
                    info!("Starting connection to {:?}:{}", host, port);

                    // Check connections_per_pkey limit
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

                    // Check for private IP addresses to prevent SSRF (unless allow_private is enabled)
                    if !self.config.allow_private {
                        let is_blocked = match &host {
                            SocksAddress::IPv4(ip) => is_private_ip(IpAddr::V4(*ip)),
                            SocksAddress::IPv6(ip) => is_private_ip(IpAddr::V6(*ip)),
                            SocksAddress::Domain(domain) => {
                                // Attempt to resolve domain and check if it resolves to a private IP
                                // We'll do a quick lookup
                                match tokio::net::lookup_host((domain.as_str(), port)).await {
                                    Ok(mut addrs) => {
                                        // Check if any resolved address is private
                                        addrs.any(|addr| is_private_ip(addr.ip()))
                                    }
                                    Err(_) => false, // If we can't resolve, let the connect fail naturally
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

                    // Connect to the target
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
                                    buffer_used: 0,
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
                    // Parse the signed connection ID
                    if connection_id_signed.len() < 8 {
                        debug!("Invalid signed connection ID length");
                        continue;
                    }

                    let sig_len = 64; // Ed25519 signature length
                    if connection_id_signed.len() < sig_len {
                        debug!("Signature too short");
                        continue;
                    }

                    // Extract signature and signed data
                    let (signed_data, _signature) =
                        connection_id_signed.split_at(connection_id_signed.len() - sig_len);

                    // The signed data should end with the connection ID (8 bytes)
                    if signed_data.len() < 8 {
                        debug!("Signed data too short");
                        continue;
                    }
                    let conn_id_start = signed_data.len() - 8;
                    let connection_id =
                        u64::from_le_bytes(signed_data[conn_id_start..].try_into().unwrap());

                    if let Some(conn_info) = client_state.connections.get_mut(&connection_id) {
                        // Update last activity
                        conn_info.last_activity = Instant::now();

                        // Write data to target
                        if let Err(e) = conn_info.stream.write_all(&data).await {
                            error!("Failed to write to target: {}", e);
                            responses.push(ServerResponseType::Close {
                                connection_id,
                                message: format!("Write error: {}", e),
                            });
                            client_state.connections.remove(&connection_id);
                        } else {
                            // Try to read response from target with configurable timeout
                            let mut buf = vec![0u8; 4096];
                            match timeout(
                                Duration::from_millis(self.config.read_timeout),
                                conn_info.stream.read(&mut buf),
                            )
                            .await
                            {
                                Ok(Ok(0)) => {
                                    // Connection closed by target
                                    responses.push(ServerResponseType::Close {
                                        connection_id,
                                        message: String::new(),
                                    });
                                    client_state.connections.remove(&connection_id);
                                }
                                Ok(Ok(n)) => {
                                    // Successfully read n > 0 bytes
                                    buf.truncate(n);
                                    conn_info.buffer_used += n;

                                    // Check buffer_max
                                    if conn_info.buffer_used > self.config.buffer_max {
                                        warn!(
                                            "Buffer limit exceeded for connection {}",
                                            connection_id
                                        );
                                        responses.push(ServerResponseType::Close {
                                            connection_id,
                                            message: "Buffer limit exceeded".to_string(),
                                        });
                                        client_state.connections.remove(&connection_id);
                                    } else {
                                        responses.push(ServerResponseType::Data {
                                            connection_id,
                                            data: buf,
                                            compressed: true, // Enable compression for responses
                                        });
                                    }
                                }
                                Ok(Err(e)) => {
                                    error!("Read error: {}", e);
                                    responses.push(ServerResponseType::Close {
                                        connection_id,
                                        message: format!("Read error: {}", e),
                                    });
                                    client_state.connections.remove(&connection_id);
                                }
                                Err(_) => {
                                    // Timeout - no immediate data available
                                    // CRITICAL FIX: Always send a response, even if empty
                                    // This prevents lambda from timing out waiting for a response
                                    debug!("No immediate response from target for connection {}, sending empty ack", connection_id);
                                    // Don't send anything - client will poll for data later
                                    // The important part is we don't leave lambda hanging
                                }
                            }
                        }
                    } else {
                        debug!("Connection ID {} not found", connection_id);
                    }
                }

                ClientMessageType::Close {
                    connection_id_signed,
                } => {
                    // Similar parsing as Data
                    let sig_len = 64;
                    if connection_id_signed.len() < sig_len {
                        debug!("Signature too short");
                        continue;
                    }

                    let (signed_data, _signature) =
                        connection_id_signed.split_at(connection_id_signed.len() - sig_len);

                    if signed_data.len() < 8 {
                        debug!("Signed data too short");
                        continue;
                    }
                    let conn_id_start = signed_data.len() - 8;
                    let connection_id =
                        u64::from_le_bytes(signed_data[conn_id_start..].try_into().unwrap());

                    if client_state.connections.remove(&connection_id).is_some() {
                        debug!("Closed connection {}", connection_id);
                        responses.push(ServerResponseType::Close {
                            connection_id,
                            message: String::new(),
                        });
                    }
                }

                ClientMessageType::Poll => {
                    debug!("Poll request - checking for pending data");
                    // Poll all active connections for data
                    let mut to_remove = Vec::new();

                    for (connection_id, conn_info) in client_state.connections.iter_mut() {
                        let mut buf = vec![0u8; 4096];
                        match timeout(Duration::from_millis(self.config.poll_timeout), conn_info.stream.read(&mut buf))
                            .await
                        {
                            Ok(Ok(0)) => {
                                // Connection closed
                                responses.push(ServerResponseType::Close {
                                    connection_id: *connection_id,
                                    message: String::new(),
                                });
                                to_remove.push(*connection_id);
                            }
                            Ok(Ok(n)) => {
                                // Successfully read n > 0 bytes
                                buf.truncate(n);
                                conn_info.buffer_used += n;
                                conn_info.last_activity = Instant::now();

                                if conn_info.buffer_used > self.config.buffer_max {
                                    warn!(
                                        "Buffer limit exceeded for connection {}",
                                        connection_id
                                    );
                                    responses.push(ServerResponseType::Close {
                                        connection_id: *connection_id,
                                        message: "Buffer limit exceeded".to_string(),
                                    });
                                    to_remove.push(*connection_id);
                                } else {
                                    responses.push(ServerResponseType::Data {
                                        connection_id: *connection_id,
                                        data: buf,
                                        compressed: true,
                                    });
                                }
                            }
                            Ok(Err(e)) => {
                                error!("Read error on connection {}: {}", connection_id, e);
                                responses.push(ServerResponseType::Close {
                                    connection_id: *connection_id,
                                    message: format!("Read error: {}", e),
                                });
                                to_remove.push(*connection_id);
                            }
                            Err(_) => {
                                // Timeout - no data, that's fine
                            }
                        }
                    }

                    // Remove closed connections
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
        debug!("Encrypting with client public key: {}", hex::encode(&client_msg.client_public_key));
        let encrypted_response = encrypt_payload(&client_msg.client_public_key, &response_data)?;
        debug!("Encrypted response: {} bytes", encrypted_response.len());
        debug!("First 32 bytes of encrypted: {:?}", &encrypted_response[..encrypted_response.len().min(32)]);

        // Send response back to lambda
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

            let mut clients = state.clients.lock().await;
            let idle_duration = Duration::from_millis(state.config.idle_timeout);

            for (_client_key, client_state) in clients.iter_mut() {
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
    // Parse args
    let args = Args::parse();

    // Initialize tracing with verbosity from args
    let log_level = args.verbosity.parse::<tracing::Level>()
        .unwrap_or(tracing::Level::INFO);

    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .without_time()
        .init();
    // Load config
    let config_str = std::fs::read_to_string(args.config)?;
    let mut config: Config = toml::from_str(&config_str)?;

    // CLI flag overrides config file
    if args.allow_private {
        config.allow_private = true;
        warn!("WARNING: Private IP access enabled (--allow-private). This should only be used for local testing!");
    }

    let bind_addr = format!("{}:{}", config.addr, config.port);
    info!("Starting server on {}", bind_addr);

    let state = Arc::new(ServerState::new(config)?);

    // Spawn cleanup task
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
