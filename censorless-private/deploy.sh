#!/bin/sh
set -e

# Check arguments
if [ -z "$1" ] || [ -z "$2" ]; then
    echo "Usage: $0 <aws-profile> <ssh-key-path>"
    echo "Example: $0 my-profile ~/.ssh/id_ed25519"
    echo ""
    echo "Environment variables:"
    echo "  RECREATE_INSTANCE=1    Destroy and recreate the EC2 instance"
    exit 1
fi

REGION=us-west-1
AWS_PROFILE=$1
SSH_KEY_PATH=$2

# Build the lambda binary
nix build .#packages.aarch64-linux.censorless-lambda

# Create temporary directory
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Copy binary to temp directory
BIN_PATH="$TEMP_DIR/censorless-v2-lambda"
cp ./result/bin/lambda $BIN_PATH

# Deploy from temp directory
cargo lambda deploy -p $AWS_PROFILE -r $REGION --enable-function-url --memory 128 --binary-path $BIN_PATH

echo ""
echo "=== Deploying NixOS server instance ==="

# Set up terraform variables
TF_VARS="-var region=$REGION -var profile=$AWS_PROFILE"

# Initialize tofu if needed
if [ ! -d ".terraform" ]; then
    echo "Initializing OpenTofu..."
    tofu init
fi

# Check if we should recreate the instance
if [ "$RECREATE_INSTANCE" = "1" ] || [ "$RECREATE_INSTANCE" = "true" ]; then
    echo "RECREATE_INSTANCE is set, destroying existing instance..."
    tofu destroy -auto-approve $TF_VARS || true
fi

# Check if instance already exists
INSTANCE_EXISTS=$(tofu show -json 2>/dev/null | grep -q "censorless_server" && echo "yes" || echo "no")

if [ "$INSTANCE_EXISTS" = "no" ]; then
    echo "Instance does not exist, creating..."
    tofu apply -auto-approve $TF_VARS
else
    echo "Instance already exists"
    tofu refresh $TF_VARS
fi

# Get server IP
SERVER_IP=$(tofu output -raw server_ip)

echo ""
echo "Server IP: $SERVER_IP"

# Wait for SSH to be available
echo ""
echo "Waiting for SSH to be available..."
MAX_RETRIES=30
RETRY_COUNT=0
while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
    if ssh -i "$SSH_KEY_PATH" -o ConnectTimeout=5 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@"$SERVER_IP" "echo SSH ready" >/dev/null 2>&1; then
        echo "SSH is ready!"
        break
    fi
    RETRY_COUNT=$((RETRY_COUNT + 1))
    echo "Waiting for SSH... (attempt $RETRY_COUNT/$MAX_RETRIES)"
    sleep 5
done

if [ $RETRY_COUNT -eq $MAX_RETRIES ]; then
    echo "Error: SSH connection timed out after $MAX_RETRIES attempts"
    exit 1
fi

# Deploy NixOS configuration to the server
echo ""
echo "=== Deploying NixOS configuration to server ==="
deploy --remote-build -s --hostname "$SERVER_IP" --ssh-opts="-i $SSH_KEY_PATH" .#censorless-server

echo ""
echo "Deployment complete!"
