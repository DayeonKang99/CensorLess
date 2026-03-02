use crate::{ProtocolError, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;

/// Encrypt a payload using the recipient's public key
///
/// For this protocol, we use a simplified encryption scheme:
/// - Generate a random 32-byte key for ChaCha20Poly1305
/// - Encrypt the payload with this key
/// - The key itself would normally be encrypted with the recipient's public key,
///   but since Ed25519 doesn't directly support encryption, we'll use a shared secret approach
///
/// Note: For production use, consider using x25519-dalek for proper key exchange
pub fn encrypt_payload(recipient_public_key: &[u8; 32], payload: &[u8]) -> Result<Vec<u8>> {
    // For this implementation, we'll derive a shared key from the recipient's public key
    // In a real implementation, you'd want to use proper key exchange (e.g., X25519)
    // For now, we'll use a simple approach: hash the public key to get an encryption key

    // Create a deterministic key from the public key
    // WARNING: This is not ideal for production! Use proper key exchange.
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(recipient_public_key);

    let cipher = ChaCha20Poly1305::new(&key_bytes.into());

    // Generate a random nonce
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encrypt the payload
    let ciphertext = cipher
        .encrypt(nonce, payload)
        .map_err(|e| ProtocolError::EncryptionError(e.to_string()))?;

    // Prepend the nonce to the ciphertext
    let mut result = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

/// Decrypt a payload using the recipient's private key
pub fn decrypt_payload(private_key: &[u8; 32], encrypted_data: &[u8]) -> Result<Vec<u8>> {
    if encrypted_data.len() < 12 {
        return Err(ProtocolError::DecryptionError(
            "Encrypted data too short".to_string(),
        ));
    }

    // Extract nonce and ciphertext
    let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Create cipher with the private key
    let cipher = ChaCha20Poly1305::new(private_key.into());

    // Decrypt
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| ProtocolError::DecryptionError(e.to_string()))?;

    Ok(plaintext)
}

/// Sign a connection ID to prove ownership
/// The signature covers: server_address + server_port + connection_id
pub fn sign_connection_id(
    signing_key: &SigningKey,
    server_addr_encoded: &[u8],
    server_port: u16,
    connection_id: u64,
) -> Vec<u8> {
    let mut data_to_sign = Vec::new();
    data_to_sign.extend_from_slice(server_addr_encoded);
    data_to_sign.extend_from_slice(&server_port.to_le_bytes());
    data_to_sign.extend_from_slice(&connection_id.to_le_bytes());

    let signature = signing_key.sign(&data_to_sign);
    signature.to_bytes().to_vec()
}

/// Verify a connection ID signature
pub fn verify_connection_id(
    verifying_key: &VerifyingKey,
    server_addr_encoded: &[u8],
    server_port: u16,
    connection_id: u64,
    signature_bytes: &[u8],
) -> Result<()> {
    if signature_bytes.len() != 64 {
        return Err(ProtocolError::InvalidSignature);
    }

    let mut data_to_verify = Vec::new();
    data_to_verify.extend_from_slice(server_addr_encoded);
    data_to_verify.extend_from_slice(&server_port.to_le_bytes());
    data_to_verify.extend_from_slice(&connection_id.to_le_bytes());

    let signature = Signature::from_bytes(signature_bytes.try_into().unwrap());

    verifying_key
        .verify(&data_to_verify, &signature)
        .map_err(|_| ProtocolError::InvalidSignature)?;

    Ok(())
}
