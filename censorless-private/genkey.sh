#!/bin/sh
# Generate Ed25519 keypair for Censorless
# Uses OpenSSL to generate keys in hex format for config files

set -e

# Generate Ed25519 private key
echo "Generating Ed25519 keypair..."
openssl genpkey -algorithm ED25519 -out /tmp/ed25519_key.pem 2>/dev/null

# Extract raw private key (32 bytes) and convert to hex
PRIVATE_KEY=$(openssl pkey -in /tmp/ed25519_key.pem -outform DER 2>/dev/null | tail -c 32 | xxd -p -c 64)

# Extract raw public key (32 bytes) and convert to hex
PUBLIC_KEY=$(openssl pkey -in /tmp/ed25519_key.pem -pubout -outform DER 2>/dev/null | tail -c 32 | xxd -p -c 64)

# Clean up temporary file
rm -f /tmp/ed25519_key.pem

echo ""
echo "=== Ed25519 Keypair Generated ==="
echo ""
echo "Private Key (use in server_config.toml or client_config.toml):"
echo "$PRIVATE_KEY"
echo ""
echo "Public Key (use in client_config.toml [[servers]] section):"
echo "$PUBLIC_KEY"
echo ""
echo "=== Configuration Examples ==="
echo ""
echo "For server_config.toml:"
echo "private_key = \"$PRIVATE_KEY\""
echo ""
echo "For client_config.toml:"
echo "private_key = \"<client's own private key>\""
echo "[[servers]]"
echo "public_key = \"$PUBLIC_KEY\""
echo "host = \"server.example.com\""
echo "port = 1337"
echo ""
