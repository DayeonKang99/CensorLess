pub mod address;
pub mod crypto;
pub mod message;

pub use address::SocksAddress;
pub use crypto::{decrypt_payload, encrypt_payload, sign_connection_id, verify_connection_id};
pub use message::{
    ClientMessage, ClientMessageType, ErrorCode, ServerResponse, ServerResponseType,
};

// Re-export ed25519-dalek types so clients don't need to depend on it directly
pub use ed25519_dalek::{SigningKey, VerifyingKey};

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid address type: {0}")]
    InvalidAddressType(u8),

    #[error("Invalid message type: {0}")]
    InvalidMessageType(u8),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Invalid public key")]
    InvalidPublicKey,

    #[error("Invalid private key")]
    InvalidPrivateKey,

    #[error("Buffer too small")]
    BufferTooSmall,

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;
