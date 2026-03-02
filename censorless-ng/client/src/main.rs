use clap::Parser;
use protocol::{
    decrypt_payload, encrypt_payload, sign_connection_id, ClientMessage, ClientMessageType,
    ErrorCode, ServerResponse, ServerResponseType, SigningKey, SocksAddress,
};
use serde::Deserialize;
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
    time::{Duration, SystemTime},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, Mutex},
    time::{interval, Instant},
};
use tracing::{debug, error, info, warn};

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
    #[allow(dead_code)]
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
    Poll {
        server_idx: usize,
    },
}

/// Shared client state
struct ClientState {
    config: Config,
    signing_key: SigningKey,
    client_public_key: [u8; 32],
    servers: Vec<ServerState>,
    outgoing_tx: mpsc::UnboundedSender<OutgoingMessage>,
    http_client: reqwest::Client,
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

        // Use current timestamp as starting nonce so client restarts don't
        // collide with the server's sliding-window state from previous sessions.
        let initial_nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

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
                nonce_counter: AtomicU64::new(initial_nonce),
                server_public_key,
            });
        }

        // Create a reusable HTTP client with connection pooling
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(4)
            .build()?;

        Ok(ClientState {
            config,
            signing_key,
            client_public_key,
            servers,
            outgoing_tx,
            http_client,
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
                                        let _ = outgoing_tx.send(OutgoingMessage::Close {
                                            server_idx,
                                            connection_id,
                                        });
                                        connections.lock().await.remove(&(server_idx, connection_id));
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

    /// Periodic cleanup task - sends Close for idle connections before removing them
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

            for (server_idx, connection_id) in &to_remove {
                let _ = state.outgoing_tx.send(OutgoingMessage::Close {
                    server_idx: *server_idx,
                    connection_id: *connection_id,
                });
                connections.remove(&(*server_idx, *connection_id));
            }

            debug!("Cleanup complete");
        }
    }

    /// Periodic poll task - sends poll messages through the outgoing channel
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

            // Send poll through the outgoing channel so nonces stay serialized
            for server_idx in servers_to_poll.keys() {
                let _ = state.outgoing_tx.send(OutgoingMessage::Poll {
                    server_idx: *server_idx,
                });
            }
        }
    }
}

/// Queue a non-StartConnection message into the pending batch.
fn queue_message(
    state: &ClientState,
    pending_messages: &mut HashMap<usize, Vec<ClientMessageType>>,
    msg: OutgoingMessage,
) {
    match msg {
        OutgoingMessage::Data {
            server_idx,
            connection_id,
            data,
        } => {
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

            pending_messages
                .entry(server_idx)
                .or_default()
                .push(ClientMessageType::Data {
                    connection_id_signed,
                    data,
                    compressed: true,
                });
        }
        OutgoingMessage::Close {
            server_idx,
            connection_id,
        } => {
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

            pending_messages
                .entry(server_idx)
                .or_default()
                .push(ClientMessageType::Close {
                    connection_id_signed,
                });
        }
        OutgoingMessage::Poll { server_idx } => {
            pending_messages
                .entry(server_idx)
                .or_default()
                .push(ClientMessageType::Poll);
        }
        OutgoingMessage::StartConnection { .. } => unreachable!(),
    }
}

/// Flush all pending messages to lambda.
async fn flush_pending(
    state: &Arc<ClientState>,
    pending_messages: &mut HashMap<usize, Vec<ClientMessageType>>,
) {
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

        if let Err(e) = send_to_lambda(state, server_idx, message, None).await {
            error!("Failed to send to lambda: {}", e);
        }
    }
}

/// Lambda sender task - serializes all outgoing messages through one task.
///
/// Strategy: drain all immediately-available messages, then flush.
/// If more messages keep arriving within `timeout`, batch them together.
/// This gives low latency for single messages while still batching bursts.
async fn lambda_sender_task(
    state: Arc<ClientState>,
    mut outgoing_rx: mpsc::UnboundedReceiver<OutgoingMessage>,
) {
    let mut pending_messages: HashMap<usize, Vec<ClientMessageType>> = HashMap::new();
    let timeout_duration = Duration::from_millis(state.config.timeout);

    loop {
        // Wait for the first message (blocking)
        let msg = match outgoing_rx.recv().await {
            Some(msg) => msg,
            None => {
                info!("Outgoing channel closed");
                break;
            }
        };

        // Handle StartConnection immediately (latency-critical)
        if matches!(msg, OutgoingMessage::StartConnection { .. }) {
            if let OutgoingMessage::StartConnection {
                server_idx,
                host,
                port,
                response_tx,
            } = msg
            {
                let server_state = &state.servers[server_idx];
                let nonce = server_state.nonce_counter.fetch_add(1, Ordering::SeqCst);

                let message = ClientMessage {
                    client_public_key: state.client_public_key,
                    nonce,
                    messages: vec![ClientMessageType::StartConnection { host, port }],
                };

                if let Err(e) =
                    send_to_lambda(&state, server_idx, message, Some(response_tx)).await
                {
                    error!("Failed to send to lambda: {}", e);
                }
            }
            continue;
        }

        // Queue the first non-StartConnection message
        queue_message(&state, &mut pending_messages, msg);

        // Drain any other immediately-available messages (non-blocking)
        loop {
            match outgoing_rx.try_recv() {
                Ok(msg) => {
                    if matches!(msg, OutgoingMessage::StartConnection { .. }) {
                        // Flush pending first, then send StartConnection immediately
                        flush_pending(&state, &mut pending_messages).await;

                        if let OutgoingMessage::StartConnection {
                            server_idx,
                            host,
                            port,
                            response_tx,
                        } = msg
                        {
                            let server_state = &state.servers[server_idx];
                            let nonce =
                                server_state.nonce_counter.fetch_add(1, Ordering::SeqCst);

                            let message = ClientMessage {
                                client_public_key: state.client_public_key,
                                nonce,
                                messages: vec![ClientMessageType::StartConnection {
                                    host,
                                    port,
                                }],
                            };

                            if let Err(e) = send_to_lambda(
                                &state,
                                server_idx,
                                message,
                                Some(response_tx),
                            )
                            .await
                            {
                                error!("Failed to send to lambda: {}", e);
                            }
                        }
                    } else {
                        queue_message(&state, &mut pending_messages, msg);
                    }
                }
                Err(_) => break, // Channel empty, stop draining
            }
        }

        // If we have pending messages, wait briefly for more to batch, then flush
        if !pending_messages.is_empty() {
            // Short wait to allow batching of concurrent messages
            tokio::select! {
                _ = tokio::time::sleep(timeout_duration) => {
                    // Timeout expired, flush what we have
                }
                msg = outgoing_rx.recv() => {
                    if let Some(msg) = msg {
                        if matches!(msg, OutgoingMessage::StartConnection { .. }) {
                            flush_pending(&state, &mut pending_messages).await;
                            if let OutgoingMessage::StartConnection { server_idx, host, port, response_tx } = msg {
                                let server_state = &state.servers[server_idx];
                                let nonce = server_state.nonce_counter.fetch_add(1, Ordering::SeqCst);
                                let message = ClientMessage {
                                    client_public_key: state.client_public_key,
                                    nonce,
                                    messages: vec![ClientMessageType::StartConnection { host, port }],
                                };
                                if let Err(e) = send_to_lambda(&state, server_idx, message, Some(response_tx)).await {
                                    error!("Failed to send to lambda: {}", e);
                                }
                            }
                        } else {
                            queue_message(&state, &mut pending_messages, msg);
                        }
                    }
                }
            }

            // Drain any remaining and flush
            loop {
                match outgoing_rx.try_recv() {
                    Ok(msg) if !matches!(msg, OutgoingMessage::StartConnection { .. }) => {
                        queue_message(&state, &mut pending_messages, msg);
                    }
                    _ => break,
                }
            }

            flush_pending(&state, &mut pending_messages).await;
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
    debug!(
        "Serialized message: {} bytes, nonce: {}",
        serialized.len(),
        message.nonce
    );
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

    // Send HTTP request to lambda using the shared client
    let response = state
        .http_client
        .post(&state.config.lambda)
        .header("Content-Type", "application/octet-stream")
        .body(request_body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("(unable to read error body)"));
        error!("Lambda HTTP error {}: {}", status, error_body);
        return Err(format!("Lambda returned HTTP {}: {}", status, error_body).into());
    }

    let response_body = response.bytes().await?;
    debug!(
        "Received response from lambda ({} bytes)",
        response_body.len()
    );

    // Decrypt the response
    let decrypted = decrypt_payload(&state.client_public_key, &response_body)?;

    // Deserialize the response
    let server_response = ServerResponse::deserialize(&decrypted)?;

    debug!("Received {} responses", server_response.responses.len());

    // Route responses to the appropriate channels
    let connections = state.connections.lock().await;

    for resp in server_response.responses {
        match &resp {
            ServerResponseType::Challenge { .. } => {
                if let Some(tx) = &response_tx {
                    let _ = tx.send(resp.clone());
                    // Don't return early - continue processing remaining responses
                }
            }
            ServerResponseType::Error { .. } => {
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

    /// Address to bind the SOCKS proxy to
    #[clap(short, long, default_value = "127.0.0.1:1080")]
    bind: String,
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
    let config: Config = toml::from_str(&config_str)?;

    info!("Starting client with lambda: {}", config.lambda);

    // Create channels for outgoing messages
    let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();

    // Create client state
    let state = Arc::new(ClientState::new(config, outgoing_tx)?);

    info!(
        "Client public key: {}",
        hex::encode(state.client_public_key)
    );

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
    let listener = TcpListener::bind(&args.bind).await?;
    let auth = Arc::new(NoAuth) as Arc<_>;
    let server = Server::new(listener, auth);

    info!("SOCKS proxy listening on {}", args.bind);

    // Set up graceful shutdown on SIGTERM/SIGINT
    let shutdown_state = state.clone();
    tokio::spawn(async move {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        let sigint = tokio::signal::ctrl_c();

        tokio::select! {
            _ = sigterm.recv() => info!("Received SIGTERM"),
            _ = sigint => info!("Received SIGINT"),
        }

        // Send Close for all active connections
        let connections = shutdown_state.connections.lock().await;
        let count = connections.len();
        if count > 0 {
            info!("Closing {} active connections...", count);
            for &(server_idx, connection_id) in connections.keys() {
                let _ = shutdown_state.outgoing_tx.send(OutgoingMessage::Close {
                    server_idx,
                    connection_id,
                });
            }
            drop(connections);
            // Give the sender a moment to flush
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        std::process::exit(0);
    });

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
