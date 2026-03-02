use crate::{address::SocksAddress, ProtocolError, Result};
use std::io::{Cursor, Read};

const COMPRESSION_THRESHOLD: usize = 128;

/// Client message types
#[derive(Debug, Clone)]
pub enum ClientMessageType {
    StartConnection {
        host: SocksAddress,
        port: u16,
    },
    Data {
        connection_id_signed: Vec<u8>,
        data: Vec<u8>,
        compressed: bool,
    },
    Close {
        connection_id_signed: Vec<u8>,
    },
    Poll, // Request pending data without sending new data
}

/// A message from the client to the server (through the lambda)
#[derive(Debug, Clone)]
pub struct ClientMessage {
    pub client_public_key: [u8; 32],
    pub nonce: u64,
    pub messages: Vec<ClientMessageType>,
}

impl ClientMessage {
    /// Serialize the client message (before encryption)
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();

        // Write client public key
        buf.extend_from_slice(&self.client_public_key);

        // Write nonce
        buf.extend_from_slice(&self.nonce.to_le_bytes());

        // Write each message
        for msg in &self.messages {
            match msg {
                ClientMessageType::StartConnection { host, port } => {
                    buf.push(0x00); // Type: start connection
                    host.encode(&mut buf)?;
                    buf.extend_from_slice(&port.to_le_bytes());
                }
                ClientMessageType::Data {
                    connection_id_signed,
                    data,
                    compressed,
                } => {
                    buf.push(0x01); // Type: data

                    // Write signed connection ID
                    if connection_id_signed.len() > 255 {
                        return Err(ProtocolError::InvalidData(
                            "Signed connection ID too long".to_string(),
                        ));
                    }
                    buf.push(connection_id_signed.len() as u8);
                    buf.extend_from_slice(connection_id_signed);

                    // Compress data if requested and beneficial
                    let (final_data, is_compressed) =
                        if *compressed && data.len() > COMPRESSION_THRESHOLD {
                            match zstd::encode_all(data.as_slice(), 3) {
                                Ok(compressed_data) if compressed_data.len() < data.len() => {
                                    (compressed_data, true)
                                }
                                _ => (data.clone(), false),
                            }
                        } else {
                            (data.clone(), false)
                        };

                    // Write compression flag
                    buf.push(if is_compressed { 0x01 } else { 0x00 });

                    // Write original data length (before compression)
                    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());

                    // Write actual data length (on wire, possibly compressed)
                    buf.extend_from_slice(&(final_data.len() as u32).to_le_bytes());

                    // Write actual data
                    buf.extend_from_slice(&final_data);
                }
                ClientMessageType::Close {
                    connection_id_signed,
                } => {
                    buf.push(0x02); // Type: close

                    // Write signed connection ID
                    if connection_id_signed.len() > 255 {
                        return Err(ProtocolError::InvalidData(
                            "Signed connection ID too long".to_string(),
                        ));
                    }
                    buf.push(connection_id_signed.len() as u8);
                    buf.extend_from_slice(connection_id_signed);
                }
                ClientMessageType::Poll => {
                    buf.push(0x03); // Type: poll (no additional data)
                }
            }
        }

        Ok(buf)
    }

    /// Deserialize a client message (after decryption)
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);

        // Read client public key
        let mut client_public_key = [0u8; 32];
        cursor.read_exact(&mut client_public_key)?;

        // Read nonce
        let mut nonce_buf = [0u8; 8];
        cursor.read_exact(&mut nonce_buf)?;
        let nonce = u64::from_le_bytes(nonce_buf);

        // Read messages
        let mut messages = Vec::new();
        while cursor.position() < data.len() as u64 {
            let mut type_buf = [0u8; 1];
            cursor.read_exact(&mut type_buf)?;
            let msg_type = type_buf[0];

            match msg_type {
                0x00 => {
                    // Start connection
                    let host = SocksAddress::decode(&mut cursor)?;
                    let mut port_buf = [0u8; 2];
                    cursor.read_exact(&mut port_buf)?;
                    let port = u16::from_le_bytes(port_buf);

                    messages.push(ClientMessageType::StartConnection { host, port });
                }
                0x01 => {
                    // Data
                    let mut len_buf = [0u8; 1];
                    cursor.read_exact(&mut len_buf)?;
                    let signed_len = len_buf[0] as usize;

                    let mut connection_id_signed = vec![0u8; signed_len];
                    cursor.read_exact(&mut connection_id_signed)?;

                    // Read compression flag
                    let mut comp_flag = [0u8; 1];
                    cursor.read_exact(&mut comp_flag)?;
                    let is_compressed = comp_flag[0] == 0x01;

                    // Read original data length
                    let mut data_len_buf = [0u8; 4];
                    cursor.read_exact(&mut data_len_buf)?;
                    let original_len = u32::from_le_bytes(data_len_buf) as usize;

                    // Read the actual data (might be more or less than original_len if compressed/decompressed)
                    // We need to read until we hit the next message type or end of data
                    // For simplicity, let's read a length field for the compressed data
                    // Actually, we need to know how much data to read. Let me reconsider the format.
                    // The format should be: compression_flag, original_length, compressed_length (if compressed), data

                    // Let's fix this: we need to know how many bytes to read
                    // We'll read the rest of the current position until we can determine the size
                    // Actually, looking at the spec, we need another length field for the actual data size

                    // For now, let's assume we read until the next message or end
                    // Better: let's add the actual data length after original length
                    let remaining = data.len() - cursor.position() as usize;

                    // We need to peek ahead to find the next message type or end
                    // This is getting complex. Let me add an actual_length field

                    // Actually, let me re-read the spec. The length field represents uncompressed data length,
                    // but we need to know how much compressed data to read.
                    // Let's add a second length field for the actual bytes on wire

                    // Reading actual data length
                    let mut actual_len_buf = [0u8; 4];
                    cursor.read_exact(&mut actual_len_buf)?;
                    let actual_len = u32::from_le_bytes(actual_len_buf) as usize;

                    let mut wire_data = vec![0u8; actual_len];
                    cursor.read_exact(&mut wire_data)?;

                    // Decompress if needed
                    let data = if is_compressed {
                        zstd::decode_all(wire_data.as_slice()).map_err(|e| {
                            ProtocolError::InvalidData(format!("Decompression failed: {}", e))
                        })?
                    } else {
                        wire_data
                    };

                    if data.len() != original_len {
                        return Err(ProtocolError::InvalidData(format!(
                            "Data length mismatch: expected {}, got {}",
                            original_len,
                            data.len()
                        )));
                    }

                    messages.push(ClientMessageType::Data {
                        connection_id_signed,
                        data,
                        compressed: is_compressed,
                    });
                }
                0x02 => {
                    // Close
                    let mut len_buf = [0u8; 1];
                    cursor.read_exact(&mut len_buf)?;
                    let signed_len = len_buf[0] as usize;

                    let mut connection_id_signed = vec![0u8; signed_len];
                    cursor.read_exact(&mut connection_id_signed)?;

                    messages.push(ClientMessageType::Close {
                        connection_id_signed,
                    });
                }
                0x03 => {
                    // Poll
                    messages.push(ClientMessageType::Poll);
                }
                _ => return Err(ProtocolError::InvalidMessageType(msg_type)),
            }
        }

        Ok(ClientMessage {
            client_public_key,
            nonce,
            messages,
        })
    }
}

/// Error codes for server error responses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    Timeout = 0x01,
    ConnectionRefused = 0x02,
    HostUnreachable = 0x03,
    TooManyConnections = 0x04,
    InvalidNonce = 0x05,
    Unknown = 0xFF,
}

impl ErrorCode {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x01 => ErrorCode::Timeout,
            0x02 => ErrorCode::ConnectionRefused,
            0x03 => ErrorCode::HostUnreachable,
            0x04 => ErrorCode::TooManyConnections,
            0x05 => ErrorCode::InvalidNonce,
            _ => ErrorCode::Unknown,
        }
    }
}

/// Server response types
#[derive(Debug, Clone)]
pub enum ServerResponseType {
    Challenge {
        connection_id: u64,
    },
    Data {
        connection_id: u64,
        data: Vec<u8>,
        compressed: bool,
    },
    Close {
        connection_id: u64,
        message: String,
    },
    Error {
        connection_id: u64, // Always 0 for errors before connection established
        error_code: ErrorCode,
        message: String,
    },
}

/// A response from the server to the client
#[derive(Debug, Clone)]
pub struct ServerResponse {
    pub responses: Vec<ServerResponseType>,
}

impl ServerResponse {
    /// Serialize the server response (before encryption)
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();

        for response in &self.responses {
            match response {
                ServerResponseType::Challenge { connection_id } => {
                    buf.push(0x00); // Type: challenge
                    buf.extend_from_slice(&connection_id.to_le_bytes());
                }
                ServerResponseType::Data {
                    connection_id,
                    data,
                    compressed,
                } => {
                    buf.push(0x01); // Type: data
                    buf.extend_from_slice(&connection_id.to_le_bytes());

                    // Compress data if requested and beneficial
                    let (final_data, is_compressed) =
                        if *compressed && data.len() > COMPRESSION_THRESHOLD {
                            match zstd::encode_all(data.as_slice(), 3) {
                                Ok(compressed_data) if compressed_data.len() < data.len() => {
                                    (compressed_data, true)
                                }
                                _ => (data.clone(), false),
                            }
                        } else {
                            (data.clone(), false)
                        };

                    // Write compression flag
                    buf.push(if is_compressed { 0x01 } else { 0x00 });

                    // Write original data length
                    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());

                    // Write actual data length
                    buf.extend_from_slice(&(final_data.len() as u32).to_le_bytes());

                    // Write data
                    buf.extend_from_slice(&final_data);
                }
                ServerResponseType::Close {
                    connection_id,
                    message,
                } => {
                    buf.push(0x02); // Type: close
                    buf.extend_from_slice(&connection_id.to_le_bytes());
                    let msg_bytes = message.as_bytes();
                    buf.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                    buf.extend_from_slice(msg_bytes);
                }
                ServerResponseType::Error {
                    connection_id,
                    error_code,
                    message,
                } => {
                    buf.push(0x03); // Type: error
                    buf.extend_from_slice(&connection_id.to_le_bytes());
                    buf.push(*error_code as u8);
                    let msg_bytes = message.as_bytes();
                    buf.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                    buf.extend_from_slice(msg_bytes);
                }
            }
        }

        Ok(buf)
    }

    /// Deserialize a server response (after decryption)
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);
        let mut responses = Vec::new();

        while cursor.position() < data.len() as u64 {
            let mut type_buf = [0u8; 1];
            cursor.read_exact(&mut type_buf)?;
            let resp_type = type_buf[0];

            match resp_type {
                0x00 => {
                    // Challenge
                    let mut conn_id_buf = [0u8; 8];
                    cursor.read_exact(&mut conn_id_buf)?;
                    let connection_id = u64::from_le_bytes(conn_id_buf);

                    responses.push(ServerResponseType::Challenge { connection_id });
                }
                0x01 => {
                    // Data
                    let mut conn_id_buf = [0u8; 8];
                    cursor.read_exact(&mut conn_id_buf)?;
                    let connection_id = u64::from_le_bytes(conn_id_buf);

                    // Read compression flag
                    let mut comp_flag = [0u8; 1];
                    cursor.read_exact(&mut comp_flag)?;
                    let is_compressed = comp_flag[0] == 0x01;

                    // Read original data length
                    let mut orig_len_buf = [0u8; 4];
                    cursor.read_exact(&mut orig_len_buf)?;
                    let original_len = u32::from_le_bytes(orig_len_buf) as usize;

                    // Read actual data length
                    let mut actual_len_buf = [0u8; 4];
                    cursor.read_exact(&mut actual_len_buf)?;
                    let actual_len = u32::from_le_bytes(actual_len_buf) as usize;

                    let mut wire_data = vec![0u8; actual_len];
                    cursor.read_exact(&mut wire_data)?;

                    // Decompress if needed
                    let data = if is_compressed {
                        zstd::decode_all(wire_data.as_slice()).map_err(|e| {
                            ProtocolError::InvalidData(format!("Decompression failed: {}", e))
                        })?
                    } else {
                        wire_data
                    };

                    if data.len() != original_len {
                        return Err(ProtocolError::InvalidData(format!(
                            "Data length mismatch: expected {}, got {}",
                            original_len,
                            data.len()
                        )));
                    }

                    responses.push(ServerResponseType::Data {
                        connection_id,
                        data,
                        compressed: is_compressed,
                    });
                }
                0x02 => {
                    // Close
                    let mut conn_id_buf = [0u8; 8];
                    cursor.read_exact(&mut conn_id_buf)?;
                    let connection_id = u64::from_le_bytes(conn_id_buf);

                    let mut len_buf = [0u8; 4];
                    cursor.read_exact(&mut len_buf)?;
                    let msg_len = u32::from_le_bytes(len_buf) as usize;

                    let mut msg_bytes = vec![0u8; msg_len];
                    cursor.read_exact(&mut msg_bytes)?;
                    let message = String::from_utf8(msg_bytes)
                        .map_err(|_| ProtocolError::InvalidData("Invalid UTF-8".to_string()))?;

                    responses.push(ServerResponseType::Close {
                        connection_id,
                        message,
                    });
                }
                0x03 => {
                    // Error
                    let mut conn_id_buf = [0u8; 8];
                    cursor.read_exact(&mut conn_id_buf)?;
                    let connection_id = u64::from_le_bytes(conn_id_buf);

                    let mut error_code_buf = [0u8; 1];
                    cursor.read_exact(&mut error_code_buf)?;
                    let error_code = ErrorCode::from_u8(error_code_buf[0]);

                    let mut len_buf = [0u8; 4];
                    cursor.read_exact(&mut len_buf)?;
                    let msg_len = u32::from_le_bytes(len_buf) as usize;

                    let mut msg_bytes = vec![0u8; msg_len];
                    cursor.read_exact(&mut msg_bytes)?;
                    let message = String::from_utf8(msg_bytes)
                        .map_err(|_| ProtocolError::InvalidData("Invalid UTF-8".to_string()))?;

                    responses.push(ServerResponseType::Error {
                        connection_id,
                        error_code,
                        message,
                    });
                }
                _ => return Err(ProtocolError::InvalidMessageType(resp_type)),
            }
        }

        Ok(ServerResponse { responses })
    }
}
