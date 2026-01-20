# Censorless-NG

A censorship-resistant proxy system that routes traffic through AWS Lambda functions, providing a layer of indirection between clients and servers to circumvent network restrictions.

## What is Censorless-NG?

Censorless-NG is a distributed proxy architecture consisting of three components:

- **Client**: A local SOCKS5 proxy that accepts connections from applications and forwards them through AWS Lambda
- **Lambda**: An AWS Lambda function that acts as a relay, forwarding encrypted traffic between clients and servers
- **Server**: A proxy gateway that connects to target hosts on behalf of clients

All communication is end-to-end encrypted using Ed25519 keys with ChaCha20Poly1305 for payload encryption. The Lambda function cannot decrypt traffic, providing privacy between client and server.

## Architecture

```
[Application] → [Client SOCKS5] → [AWS Lambda] → [Server] → [Target Host]
                    |                             |
                    └──────── Encrypted ──────────┘
```

See `docs/DESIGN.md` for complete protocol specifications.

## Prerequisites

### With Nix (Recommended)

- [Nix package manager](https://nixos.org/download.html) with flakes enabled

### Without Nix

- Rust toolchain (1.70+)
- cargo
- cargo-lambda (for building Lambda function): `cargo install cargo-lambda`
- pkg-config
- OpenSSL development libraries

## Building

### With Nix

The Nix flake provides a reproducible development environment with all dependencies:

```bash
# Build all components
nix build .#

# Enter development shell
nix develop

# Or run cargo commands directly
nix develop -c cargo build --workspace --release
```

### Without Nix

```bash
# Build all components
cargo build --workspace --release

# Build specific component
cargo build -p client --release
cargo build -p server --release

# Build Lambda function (requires cargo-lambda)
cargo lambda build --release -p lambda
```

## Configuration

client.toml and server.toml provide example configurations for the client and server;

### Lambda Configuration

Configure Lambda function with environment variables:

- `CLIENT_WHITELIST` (optional): JSON array of allowed client public keys
- `SERVER_WHITELIST` (optional): JSON array of allowed servers
- `TIMEOUT` (optional): Server response timeout in milliseconds (default: 5000)

Example:
```json
CLIENT_WHITELIST=["client1_pubkey_hex", "client2_pubkey_hex"]
SERVER_WHITELIST=[{"host": "server.example.com", "port": 1337, "public_key": "server_pubkey_hex"}]
```

## Running

### With Nix

```bash
# Run client
nix develop -c cargo run -p client -- --config client.toml

# Run server
nix develop -c cargo run -p server -- --config server.toml

# Run server with private IP access (for local testing)
nix develop -c cargo run -p server -- --config server.toml --allow-private

# Or in development shell
nix develop
cargo run -p client -- --config client.toml
cargo run -p server -- --config server.toml
```

### Without Nix

```bash
# Run client
cargo run --release -p client -- --config client.toml

# Run server
cargo run --release -p server -- --config server.toml

# Run server with private IP access (for local testing)
cargo run --release -p server -- --config server.toml --allow-private
```

### Deploying Lambda

After building the Lambda function:

```bash
# With Nix
nix develop -c cargo lambda build --release -p lambda --target aarch64-unknown-linux-gnu

# Without Nix
cargo lambda build --release -p lambda --target aarch64-unknown-linux-gnu

# Deploy using cargo-lambda
cargo lambda deploy --region us-east-1

# Or package for manual deployment
cargo lambda build --release -p lambda --output-format zip
```

Upload the resulting ZIP file to AWS Lambda and configure a Function URL.

## Testing

```bash
# With Nix
nix develop -c cargo test --workspace

# Without Nix
cargo test --workspace

# Test specific component
cargo test -p protocol
```

## Key Generation

Generate Ed25519 key pairs for clients and servers:
```bash
./genkey.sh
```

## Security Considerations
- **SSRF Protection**: The server blocks connections to private IP ranges by default. Use `--allow-private` only for local testing.
- **Key Management**: Keep private keys secure. Never commit them to version control.
- **Whitelisting**: Use Lambda's `CLIENT_WHITELIST` and `SERVER_WHITELIST` to restrict access.
- **Rate Limiting**: Server enforces connection limits per client public key.

## Development

```bash
# Format code
cargo fmt --all

# Run linter
cargo clippy --workspace

# Check formatting
cargo fmt --all -- --check
```

## Documentation
- `docs/DESIGN.md`: Complete protocol specification
