use clap::Parser;
use protocol::{
    decrypt_payload, encrypt_payload, sign_connection_id, ClientMessage, ClientMessageType,
    ErrorCode, ServerResponse, ServerResponseType, SigningKey, SocksAddress,
};
use serde::Deserialize;
use tracing::{debug, error, info, warn};
use socks5_server::{
    auth::NoAuth,
    connection::state::NeedAuthenticate,
    proto::{Address, Error as SocksError, Reply},
    Command, IncomingConnection, Server,
};
use std::path::PathBuf;
use std::{
    collections::HashMap,
    io::Error as IoError,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, Mutex},
    time::{interval, Instant},
};

#[derive(Debug, Deserialize, Clone)]
struct ServerConfig {
    public_key: String,
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
struct Config {
    private_key: String,
    lambda: String,
    lambda_buffer: usize,
    timeout: u64,
    idle_timeout: u64,
    servers: Vec<ServerConfig>,
}

/// Per-server state
struct ServerState {
    config: ServerConfig,
    nonce_counter: AtomicU64,
    server_public_key: [u8; 32],
}

/// Connection tracking info
struct ConnectionInfo {
    last_activity: Instant,
    response_tx: mpsc::UnboundedSender<ServerResponseType>,
}

/// Message from SOCKS handler to lambda sender
enum OutgoingMessage {
    StartConnection {
        server_idx: usize,
        host: SocksAddress,
        port: u16,
        response_tx: mpsc::UnboundedSender<ServerResponseType>,
    },
    Data {
        server_idx: usize,
        connection_id: u64,
        data: Vec<u8>,
    },
    Close {
        server_idx: usize,
        connection_id: u64,
    },
}

/// Shared client state
struct ClientState {
    config: Config,
    signing_key: SigningKey,
    client_public_key: [u8; 32],
    servers: Vec<ServerState>,
    outgoing_tx: mpsc::UnboundedSender<OutgoingMessage>,
    // Map from (server_idx, connection_id) to connection info
    connections: Arc<Mutex<HashMap<(usize, u64), ConnectionInfo>>>,
}

impl ClientState {
    fn new(
        config: Config,
        outgoing_tx: mpsc::UnboundedSender<OutgoingMessage>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Parse private key
        let private_key_bytes = hex::decode(&config.private_key)?;
        if private_key_bytes.len() != 32 {
            return Err("Invalid private key length".into());
        }
        let mut private_key = [0u8; 32];
        private_key.copy_from_slice(&private_key_bytes);

        let signing_key = SigningKey::from_bytes(&private_key);
        let client_public_key = signing_key.verifying_key().to_bytes();

        // Parse server public keys
        let mut servers = Vec::new();
        for server_config in &config.servers {
            let public_key_bytes = hex::decode(&server_config.public_key)?;
            if public_key_bytes.len() != 32 {
                return Err("Invalid server public key length".into());
            }
            let mut server_public_key = [0u8; 32];
            server_public_key.copy_from_slice(&public_key_bytes);

            servers.push(ServerState {
                config: server_config.clone(),
                nonce_counter: AtomicU64::new(1),
                server_public_key,
            });
        }

        Ok(ClientState {
            config,
            signing_key,
            client_public_key,
            servers,
            outgoing_tx,
            connections: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn handle_socks_connection(
        &self,
        conn: IncomingConnection<(), NeedAuthenticate>,
    ) -> Result<(), SocksError> {
        // Authenticate
        let conn = match conn.authenticate().await {
            Ok((conn, _)) => conn,
            Err((err, mut conn)) => {
                let _ = conn.shutdown().await;
                return Err(err);
            }
        };

        // Wait for command
        match conn.wait().await {
            Ok(Command::Connect(connect, addr)) => {
                // For now, use the first server
                let server_idx = 0;

                // Convert SOCKS address
                let (host, port) = match addr {
                    Address::DomainAddress(domain, port) => {
                        let domain = String::from_utf8_lossy(&domain).to_string();
                        (SocksAddress::Domain(domain), port)
                    }
                    Address::SocketAddress(addr) => match addr {
                        std::net::SocketAddr::V4(v4) => (SocksAddress::IPv4(*v4.ip()), v4.port()),
                        std::net::SocketAddr::V6(v6) => (SocksAddress::IPv6(*v6.ip()), v6.port()),
                    },
                };

                info!("SOCKS CONNECT to {:?}:{}", host, port);

                // Create response channel
                let (response_tx, mut response_rx) = mpsc::unbounded_channel();

                // Send start connection message
                self.outgoing_tx
                    .send(OutgoingMessage::StartConnection {
                        server_idx,
                        host,
                        port,
                        response_tx: response_tx.clone(),
                    })
                    .map_err(|_| SocksError::Io(IoError::other("Failed to send message")))?;

                // Wait for challenge or error response
                let connection_id = match response_rx.recv().await {
                    Some(ServerResponseType::Challenge { connection_id }) => connection_id,
                    Some(ServerResponseType::Error {
                        error_code,
                        message,
                        ..
                    }) => {
                        warn!("Connection failed: {:?} - {}", error_code, message);
                        let reply = match error_code {
                            ErrorCode::ConnectionRefused => Reply::ConnectionRefused,
                            ErrorCode::HostUnreachable => Reply::HostUnreachable,
                            ErrorCode::Timeout => Reply::TtlExpired,
                            _ => Reply::GeneralFailure,
                        };
                        let replied = connect.reply(reply, Address::unspecified()).await;
                        if let Ok(mut conn) = replied {
                            let _ = conn.shutdown().await;
                        }
                        return Ok(());
                    }
                    Some(ServerResponseType::Close { message, .. }) => {
                        warn!("Connection closed: {}", message);
                        let replied = connect
                            .reply(Reply::HostUnreachable, Address::unspecified())
                            .await;
                        if let Ok(mut conn) = replied {
                            let _ = conn.shutdown().await;
                        }
                        return Ok(());
                    }
                    _ => {
                        warn!("Unexpected response or timeout");
                        let replied = connect
                            .reply(Reply::GeneralFailure, Address::unspecified())
                            .await;
                        if let Ok(mut conn) = replied {
                            let _ = conn.shutdown().await;
                        }
                        return Ok(());
                    }
                };

                info!("Connection established with ID: {}", connection_id);

                // Register connection
                self.connections.lock().await.insert(
                    (server_idx, connection_id),
                    ConnectionInfo {
                        last_activity: Instant::now(),
                        response_tx: response_tx.clone(),
                    },
                );

                // Send success reply
                let replied = connect
                    .reply(Reply::Succeeded, Address::unspecified())
                    .await;

                let mut conn = match replied {
                    Ok(conn) => conn,
                    Err((err, mut conn)) => {
                        let _ = conn.shutdown().await;
                        return Err(SocksError::Io(err));
                    }
                };

                // Spawn task to handle bidirectional data transfer
                let outgoing_tx = self.outgoing_tx.clone();
                let connections = self.connections.clone();

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    loop {
                        tokio::select! {
                            // Read from SOCKS connection
                            read_result = conn.read(&mut buf) => {
                                match read_result {
                                    Ok(0) => {
                                        debug!("SOCKS connection closed");
                                        let _ = outgoing_tx.send(OutgoingMessage::Close {
                                            server_idx,
                                            connection_id,
                                        });
                                        connections.lock().await.remove(&(server_idx, connection_id));
                                        break;
                                    }
                                    Ok(n) => {
                                        debug!("Read {} bytes from SOCKS", n);
                                        let data = buf[..n].to_vec();

                                        // Update last activity
                                        if let Some(conn_info) = connections.lock().await.get_mut(&(server_idx, connection_id)) {
                                            conn_info.last_activity = Instant::now();
                                        }

                                        let _ = outgoing_tx.send(OutgoingMessage::Data {
                                            server_idx,
                                            connection_id,
                                            data,
                                        });
                                    }
                                    Err(e) => {
                                        error!("Error reading from SOCKS: {}", e);
                                        break;
                                    }
                                }
                            }
                            // Receive responses from server
                            response = response_rx.recv() => {
                                match response {
                                    Some(ServerResponseType::Data { data, .. }) => {
                                        debug!("Writing {} bytes to SOCKS", data.len());

                                        // Update last activity
                                        if let Some(conn_info) = connections.lock().await.get_mut(&(server_idx, connection_id)) {
                                            conn_info.last_activity = Instant::now();
                                        }

                                        if let Err(e) = conn.write_all(&data).await {
                                            error!("Error writing to SOCKS: {}", e);
                                            break;
                                        }
                                    }
                                    Some(ServerResponseType::Close { message, .. }) => {
                                        if !message.is_empty() {
                                            debug!("Connection closed: {}", message);
                                        }
                                        connections.lock().await.remove(&(server_idx, connection_id));
                                        break;
                                    }
                                    None => {
                                        debug!("Response channel closed");
                                        connections.lock().await.remove(&(server_idx, connection_id));
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    let _ = conn.shutdown().await;
                });
            }
            Ok(Command::Associate(associate, _)) => {
                let replied = associate
                    .reply(Reply::CommandNotSupported, Address::unspecified())
                    .await;
                if let Ok(mut conn) = replied {
                    let _ = conn.close().await;
                }
            }
            Ok(Command::Bind(bind, _)) => {
                let replied = bind
                    .reply(Reply::CommandNotSupported, Address::unspecified())
                    .await;
                if let Ok(mut conn) = replied {
                    let _ = conn.close().await;
                }
            }
            Err((err, mut conn)) => {
                let _ = conn.shutdown().await;
                return Err(err);
            }
        }

        Ok(())
    }

    /// Periodic cleanup task
    async fn cleanup_task(state: Arc<ClientState>) {
        let mut cleanup_interval = interval(Duration::from_secs(60));

        loop {
            cleanup_interval.tick().await;
            debug!("Running cleanup task");

            let mut connections = state.connections.lock().await;
            let idle_duration = Duration::from_millis(state.config.idle_timeout);
            let mut to_remove = Vec::new();

            for (key, conn_info) in connections.iter() {
                if conn_info.last_activity.elapsed() > idle_duration {
                    debug!("Removing idle connection: {:?}", key);
                    to_remove.push(*key);
                }
            }

            for key in to_remove {
                connections.remove(&key);
            }

            debug!("Cleanup complete");
        }
    }

    /// Periodic poll task - sends poll messages to check for pending data
    async fn poll_task(state: Arc<ClientState>) {
        let mut poll_interval = interval(Duration::from_millis(state.config.timeout / 2));

        loop {
            poll_interval.tick().await;

            let connections = state.connections.lock().await;
            if connections.is_empty() {
                continue;
            }

            // Group connections by server
            let mut servers_to_poll: HashMap<usize, bool> = HashMap::new();
            for (server_idx, _) in connections.keys() {
                servers_to_poll.insert(*server_idx, true);
            }

            drop(connections);

            // Send poll message to each server with active connections
            for server_idx in servers_to_poll.keys() {
                let server_state = &state.servers[*server_idx];
                let nonce = server_state.nonce_counter.fetch_add(1, Ordering::SeqCst);

                let message = ClientMessage {
                    client_public_key: state.client_public_key,
                    nonce,
                    messages: vec![ClientMessageType::Poll],
                };

                if let Err(e) = send_to_lambda(&state, *server_idx, message, None).await {
                    error!("Failed to send poll to lambda: {}", e);
                }
            }
        }
    }
}

/// Lambda sender task
async fn lambda_sender_task(
    state: Arc<ClientState>,
    mut outgoing_rx: mpsc::UnboundedReceiver<OutgoingMessage>,
) {
    let mut pending_messages: HashMap<usize, Vec<ClientMessageType>> = HashMap::new();
    let mut last_send = Instant::now();
    let timeout_duration = Duration::from_millis(state.config.timeout);

    let mut ticker = interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            msg = outgoing_rx.recv() => {
                match msg {
                    Some(OutgoingMessage::StartConnection { server_idx, host, port, response_tx }) => {
                        let server_state = &state.servers[server_idx];
                        let nonce = server_state.nonce_counter.fetch_add(1, Ordering::SeqCst);

                        let message = ClientMessage {
                            client_public_key: state.client_public_key,
                            nonce,
                            messages: vec![ClientMessageType::StartConnection { host, port }],
                        };

                        // Send immediately and pass response_tx for error handling
                        if let Err(e) = send_to_lambda(&state, server_idx, message, Some(response_tx)).await {
                            error!("Failed to send to lambda: {}", e);
                        }
                    }
                    Some(OutgoingMessage::Data { server_idx, connection_id, data }) => {
                        // Create signed connection ID
                        let server_config = &state.servers[server_idx].config;
                        let mut server_addr_buf = Vec::new();
                        let server_addr = SocksAddress::Domain(server_config.host.clone());
                        server_addr.encode(&mut server_addr_buf).unwrap();

                        let signature = sign_connection_id(
                            &state.signing_key,
                            &server_addr_buf,
                            server_config.port,
                            connection_id,
                        );

                        let mut connection_id_signed = Vec::new();
                        connection_id_signed.extend_from_slice(&server_addr_buf);
                        connection_id_signed.extend_from_slice(&server_config.port.to_le_bytes());
                        connection_id_signed.extend_from_slice(&connection_id.to_le_bytes());
                        connection_id_signed.extend_from_slice(&signature);

                        let msg = ClientMessageType::Data {
                            connection_id_signed,
                            data,
                            compressed: true, // Enable compression
                        };

                        pending_messages.entry(server_idx).or_default().push(msg);
                    }
                    Some(OutgoingMessage::Close { server_idx, connection_id }) => {
                        // Create signed connection ID
                        let server_config = &state.servers[server_idx].config;
                        let mut server_addr_buf = Vec::new();
                        let server_addr = SocksAddress::Domain(server_config.host.clone());
                        server_addr.encode(&mut server_addr_buf).unwrap();

                        let signature = sign_connection_id(
                            &state.signing_key,
                            &server_addr_buf,
                            server_config.port,
                            connection_id,
                        );

                        let mut connection_id_signed = Vec::new();
                        connection_id_signed.extend_from_slice(&server_addr_buf);
                        connection_id_signed.extend_from_slice(&server_config.port.to_le_bytes());
                        connection_id_signed.extend_from_slice(&connection_id.to_le_bytes());
                        connection_id_signed.extend_from_slice(&signature);

                        let msg = ClientMessageType::Close { connection_id_signed };
                        pending_messages.entry(server_idx).or_default().push(msg);
                    }
                    None => {
                        info!("Outgoing channel closed");
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                // Check if we should flush
                let should_flush = last_send.elapsed() >= timeout_duration;

                if should_flush && !pending_messages.is_empty() {
                    for (server_idx, messages) in pending_messages.drain() {
                        if messages.is_empty() {
                            continue;
                        }

                        let server_state = &state.servers[server_idx];
                        let nonce = server_state.nonce_counter.fetch_add(1, Ordering::SeqCst);

                        let message = ClientMessage {
                            client_public_key: state.client_public_key,
                            nonce,
                            messages,
                        };

                        if let Err(e) = send_to_lambda(&state, server_idx, message, None).await {
                            error!("Failed to send to lambda: {}", e);
                        }
                    }

                    last_send = Instant::now();
                }
            }
        }
    }
}

async fn send_to_lambda(
    state: &Arc<ClientState>,
    server_idx: usize,
    message: ClientMessage,
    response_tx: Option<mpsc::UnboundedSender<ServerResponseType>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let server_config = &state.servers[server_idx].config;
    let server_public_key = &state.servers[server_idx].server_public_key;

    // Serialize and encrypt the message
    let serialized = message.serialize()?;
    debug!("Serialized message: {} bytes, nonce: {}", serialized.len(), message.nonce);
    let encrypted = encrypt_payload(server_public_key, &serialized)?;
    debug!("Encrypted to: {} bytes", encrypted.len());

    // Build the request body: server_addr + server_port + length + encrypted_payload
    let mut request_body = Vec::new();

    let server_addr = SocksAddress::Domain(server_config.host.clone());
    server_addr.encode(&mut request_body)?;
    request_body.extend_from_slice(&server_config.port.to_le_bytes());
    request_body.extend_from_slice(&(encrypted.len() as u32).to_le_bytes());
    request_body.extend_from_slice(&encrypted);

    debug!("Sending request to lambda ({} bytes)", request_body.len());
    debug!("First 32 bytes: {:?}", &request_body[..request_body.len().min(32)]);

    // Send HTTP request to lambda
    let client = reqwest::Client::new();
    let response = client
        .post(&state.config.lambda)
        .header("Content-Type", "application/octet-stream")
        .body(request_body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        // Try to read the error message from the response body
        let error_body = response.text().await.unwrap_or_else(|_| String::from("(unable to read error body)"));
        error!("Lambda HTTP error {}: {}", status, error_body);
        return Err(format!("Lambda returned HTTP {}: {}", status, error_body).into());
    }

    let response_body = response.bytes().await?;
    debug!(
        "Received response from lambda ({} bytes)",
        response_body.len()
    );
    debug!("First 32 bytes of response: {:?}", &response_body[..response_body.len().min(32)]);

    // Decrypt the response (using public key to match what server encrypted with)
    debug!("Decrypting response with client public key: {}", hex::encode(&state.client_public_key));
    debug!("About to decrypt {} bytes", response_body.len());
    let decrypted = decrypt_payload(&state.client_public_key, &response_body)?;
    debug!("Decryption successful");

    // Deserialize the response
    let server_response = ServerResponse::deserialize(&decrypted)?;

    debug!("Received {} responses", server_response.responses.len());

    // Route responses to the appropriate channels
    let connections = state.connections.lock().await;

    for resp in server_response.responses {
        match &resp {
            ServerResponseType::Challenge { connection_id } => {
                if let Some(tx) = &response_tx {
                    // This is a response to a StartConnection
                    let _ = tx.send(resp.clone());
                    // Connection will be registered when SOCKS handler receives the challenge
                    return Ok(());
                }
            }
            ServerResponseType::Error { .. } => {
                // Send error to the response_tx if available (for StartConnection errors)
                if let Some(tx) = &response_tx {
                    let _ = tx.send(resp.clone());
                }
            }
            ServerResponseType::Data { connection_id, .. }
            | ServerResponseType::Close { connection_id, .. } => {
                if let Some(conn_info) = connections.get(&(server_idx, *connection_id)) {
                    let _ = conn_info.response_tx.send(resp);
                }
            }
        }
    }

    Ok(())
}
#[derive(Debug, Parser)]
struct Args {
    #[clap(short, long, default_value = "client.toml")]
    config: PathBuf,

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
    let config: Config = toml::from_str(&config_str)?;

    info!("Starting client with lambda: {}", config.lambda);

    // Create channels for outgoing messages
    let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();

    // Create client state
    let state = Arc::new(ClientState::new(config, outgoing_tx)?);

    info!("Client public key: {}", hex::encode(&state.client_public_key));

    // Spawn lambda sender task
    let sender_state = state.clone();
    tokio::spawn(async move {
        lambda_sender_task(sender_state, outgoing_rx).await;
    });

    // Spawn cleanup task
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        ClientState::cleanup_task(cleanup_state).await;
    });

    // Spawn poll task
    let poll_state = state.clone();
    tokio::spawn(async move {
        ClientState::poll_task(poll_state).await;
    });

    // Start SOCKS server
    let listener = TcpListener::bind("127.0.0.1:1080").await?;
    let auth = Arc::new(NoAuth) as Arc<_>;
    let server = Server::new(listener, auth);

    info!("SOCKS proxy listening on 127.0.0.1:1080");

    while let Ok((conn, _)) = server.accept().await {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = state.handle_socks_connection(conn).await {
                error!("Error handling SOCKS connection: {}", e);
            }
        });
    }

    Ok(())
}
