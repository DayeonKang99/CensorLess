use lambda_http::tracing::init_default_subscriber;
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use protocol::SocksAddress;
use serde::Deserialize;
use std::io::Cursor;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Debug, Deserialize, Clone)]
struct ServerWhitelistEntry {
    host: String,
    port: u16,
    public_key: String,
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    client_whitelist: Option<Vec<String>>, // List of hex-encoded client public keys
    server_whitelist: Option<Vec<ServerWhitelistEntry>>,
    timeout: u64, // milliseconds - timeout for connecting to server
    read_timeout: u64, // milliseconds - timeout for reading response from server (should be > server's read_timeout + network overhead)
}

impl Config {
    fn from_env() -> Self {
        // Try to read config from environment variables
        // For AWS Lambda, these would be set in the Lambda configuration

        let client_whitelist = std::env::var("CLIENT_WHITELIST")
            .ok()
            .map(|s| serde_json::from_str(&s).unwrap_or_default());

        let server_whitelist = std::env::var("SERVER_WHITELIST")
            .ok()
            .map(|s| serde_json::from_str(&s).unwrap_or_default());

        let timeout = std::env::var("TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5000); // Default 5 seconds

        let read_timeout = std::env::var("READ_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(7000); // Default 7 seconds (2s buffer over default server read_timeout of 1s + 4s network buffer)

        Config {
            client_whitelist,
            server_whitelist,
            timeout,
            read_timeout,
        }
    }
}

/// Inner handler that uses ? for error propagation
async fn handle_request(event: Request) -> Result<Response<Body>, Box<dyn std::error::Error>> {
    tracing::info!(
        "Lambda function invoked, method: {}, path: {}",
        event.method(),
        event.uri().path()
    );

    // Load config
    let config = Config::from_env();
    tracing::debug!(
        "Config loaded: connect_timeout={}ms, read_timeout={}ms, client_whitelist={}, server_whitelist={}",
        config.timeout,
        config.read_timeout,
        config
            .client_whitelist
            .as_ref()
            .map(|w| w.len())
            .unwrap_or(0),
        config
            .server_whitelist
            .as_ref()
            .map(|w| w.len())
            .unwrap_or(0)
    );

    // Extract the HTTP body
    let body_bytes = match event.body() {
        Body::Empty => {
            tracing::warn!("Received empty request body");
            return Err("Empty request body".into());
        }
        Body::Text(t) => {
            tracing::debug!("Received text body, {} bytes", t.len());
            t.as_bytes().to_vec()
        }
        Body::Binary(b) => {
            tracing::debug!("Received binary body, {} bytes", b.len());
            tracing::trace!("First 32 bytes: {:?}", &b[..b.len().min(32)]);
            b.clone()
        }
    };

    // Parse the request body
    // Format: <server_address> <server_port> <length> <encrypted_payload>
    let mut cursor = Cursor::new(body_bytes.as_slice());

    // Decode server address
    let server_addr = SocksAddress::decode(&mut cursor)
        .map_err(|e| format!("Failed to decode server address: {}", e))?;
    tracing::debug!("Decoded server address: {:?}", server_addr);

    // Decode server port
    let mut port_buf = [0u8; 2];
    std::io::Read::read_exact(&mut cursor, &mut port_buf)
        .map_err(|e| format!("Failed to read server port: {}", e))?;
    let server_port = u16::from_le_bytes(port_buf);
    tracing::debug!("Server port: {}", server_port);

    // Decode payload length
    let mut len_buf = [0u8; 4];
    std::io::Read::read_exact(&mut cursor, &mut len_buf)
        .map_err(|e| format!("Failed to read payload length: {}", e))?;
    let payload_len = u32::from_le_bytes(len_buf) as usize;
    tracing::debug!("Payload length: {} bytes", payload_len);

    // Read the encrypted payload
    let mut encrypted_payload = vec![0u8; payload_len];
    std::io::Read::read_exact(&mut cursor, &mut encrypted_payload).map_err(|e| {
        format!(
            "Failed to read encrypted payload (expected {} bytes, cursor at position {}): {}",
            payload_len,
            cursor.position(),
            e
        )
    })?;
    tracing::debug!("Successfully read encrypted payload");

    // Check server whitelist if configured
    if let Some(ref whitelist) = config.server_whitelist {
        let server_addr_str = match &server_addr {
            SocksAddress::IPv4(ip) => ip.to_string(),
            SocksAddress::IPv6(ip) => ip.to_string(),
            SocksAddress::Domain(d) => d.clone(),
        };

        let is_whitelisted = whitelist
            .iter()
            .any(|entry| entry.host == server_addr_str && entry.port == server_port);

        if !is_whitelisted {
            tracing::warn!(
                "Server not in whitelist: {}:{}",
                server_addr_str,
                server_port
            );
            return Err(format!(
                "Server not in whitelist: {}:{}",
                server_addr_str, server_port
            )
            .into());
        }
        tracing::debug!("Server is whitelisted");
    }

    // If client whitelist is enabled, we'd need to decrypt the payload first to get the client public key
    // For now, we'll skip this and just check if the whitelist is configured
    // A proper implementation would require partially decrypting to extract the client public key

    // Connect to the server
    let target_addr = match server_addr {
        SocksAddress::IPv4(ip) => format!("{}:{}", ip, server_port),
        SocksAddress::IPv6(ip) => format!("[{}]:{}", ip, server_port),
        SocksAddress::Domain(domain) => format!("{}:{}", domain, server_port),
    };

    tracing::info!("Connecting to server: {}", target_addr);

    let mut stream = timeout(
        Duration::from_millis(config.timeout),
        TcpStream::connect(&target_addr),
    )
    .await
    .map_err(|_| format!("Connection to server timed out after {}ms", config.timeout))?
    .map_err(|e| format!("Failed to connect to server {}: {}", target_addr, e))?;

    tracing::info!("Successfully connected to server");
    tracing::debug!("Sending payload of {} bytes", encrypted_payload.len());

    // Send the encrypted payload to the server
    stream
        .write_all(&encrypted_payload)
        .await
        .map_err(|e| format!("Failed to write to server: {}", e))?;

    tracing::debug!("Wrote {} bytes successfully", encrypted_payload.len());

    // Shutdown the write half to signal we're done sending
    if let Err(e) = stream.shutdown().await {
        tracing::warn!("Failed to shutdown write half: {}", e);
    } else {
        tracing::debug!("Shutdown write half successfully");
    }

    tracing::debug!("Payload sent, waiting for response");

    // Read the response from the server
    // Use read_timeout which should be larger than server's processing time + network overhead
    let mut response_buf = Vec::new();
    match timeout(
        Duration::from_millis(config.read_timeout),
        stream.read_to_end(&mut response_buf),
    )
    .await
    {
        Ok(result) => {
            result.map_err(|e| format!("Failed to read from server: {}", e))?;
            tracing::info!("Received response of {} bytes", response_buf.len());
        }
        Err(_) => {
            tracing::warn!(
                "Read from server timed out after {}ms, received {} bytes so far",
                config.read_timeout,
                response_buf.len()
            );
            // Always return an error on timeout — partial encrypted data cannot
            // be decrypted by the client and would cause silent failures.
            return Err(format!(
                "Server response timed out after {}ms ({} bytes received)",
                config.read_timeout,
                response_buf.len()
            )
            .into());
        }
    }

    // Return the response to the client
    tracing::debug!("Returning {} bytes to client", response_buf.len());

    if response_buf.is_empty() {
        tracing::error!("Server returned 0 bytes (connection likely closed without response)");
        return Err("Server closed connection without sending response".into());
    }

    tracing::trace!(
        "First 32 bytes: {:?}",
        &response_buf[..response_buf.len().min(32)]
    );
    tracing::info!("Returning successful response");
    Ok(Response::builder()
        .status(200)
        .header("Content-Type", "application/octet-stream")
        .body(Body::Binary(response_buf))?)
}

/// Wrapper handler that converts errors to HTTP responses
async fn function_handler(event: Request) -> Result<Response<Body>, Error> {
    tracing::info!("Handler called, about to process request");

    match handle_request(event).await {
        Ok(response) => {
            tracing::info!("Request succeeded");
            Ok(response)
        }
        Err(e) => {
            let error_msg = e.to_string();
            tracing::error!("Request failed: {}", error_msg);
            Ok(Response::builder()
                .status(500)
                .body(Body::Text(error_msg))?)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Use the default logger (aws)
    init_default_subscriber();
    let result = run(service_fn(function_handler)).await;
    if let Err(ref e) = result {
        tracing::error!("Lambda runtime error: {:?}", e);
    }
    result
}
