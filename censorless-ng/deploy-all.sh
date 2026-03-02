#!/bin/sh
# Deploy all censorless-ng components (server, lambda, optionally client)
# Usage:
#   ./deploy-all.sh local          - Build and run locally
#   ./deploy-all.sh aws <profile> <ssh-key>  - Deploy to AWS
#   ./deploy-all.sh stop           - Stop local processes

set -e

PIDFILE="/tmp/censorless-local.pids"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

usage() {
    echo "Usage: $0 <mode> [options]"
    echo ""
    echo "Modes:"
    echo "  local                    Build and run server + lambda locally"
    echo "  aws <profile> <ssh-key>  Deploy lambda + server to AWS"
    echo "  stop                     Stop locally running processes"
    exit 1
}

# Wait for a TCP port to be listening (timeout after 30s)
wait_for_port() {
    port=$1
    label=$2
    attempts=0
    max_attempts=60
    while [ $attempts -lt $max_attempts ]; do
        if nc -z 127.0.0.1 "$port" 2>/dev/null; then
            echo "  $label is ready on port $port"
            return 0
        fi
        attempts=$((attempts + 1))
        sleep 0.5
    done
    echo "  ERROR: $label did not start on port $port within 30s"
    return 1
}

mode_local() {
    echo "=== Building workspace ==="
    cargo build --workspace

    echo ""
    echo "=== Starting server ==="
    cargo run -p server -- --config server.toml --allow-private -v debug &
    SERVER_PID=$!

    echo "=== Starting lambda (cargo lambda watch) ==="
    # Higher read timeout for local: server processes messages sequentially,
    # so batches with multiple Data/Poll messages need more time
    READ_TIMEOUT=30000 cargo lambda watch &
    LAMBDA_PID=$!

    # Write pidfile
    echo "$SERVER_PID $LAMBDA_PID" > "$PIDFILE"

    echo ""
    echo "Waiting for services to start..."
    wait_for_port 1337 "Server"
    wait_for_port 9000 "Lambda"

    echo ""
    echo "=== All services ready ==="
    echo "  Server PID: $SERVER_PID (port 1337)"
    echo "  Lambda PID: $LAMBDA_PID (port 9000)"
    echo "  PID file:   $PIDFILE"
    echo ""
    echo "Run './deploy-all.sh stop' to shut down."
}

mode_aws() {
    if [ -z "$1" ] || [ -z "$2" ]; then
        echo "Error: aws mode requires <profile> and <ssh-key>"
        echo "Usage: $0 aws <profile> <ssh-key>"
        exit 1
    fi

    AWS_PROFILE=$1
    SSH_KEY_PATH=$2

    echo "=== Deploying to AWS (profile: $AWS_PROFILE) ==="

    # Run the existing deploy script
    sh deploy.sh "$AWS_PROFILE" "$SSH_KEY_PATH"

    # Get outputs
    SERVER_IP=$(tofu output -raw server_ip 2>/dev/null || echo "unknown")
    LAMBDA_URL=$(aws --profile "$AWS_PROFILE" lambda get-function-url-config \
        --function-name censorless-v2-lambda \
        --query FunctionUrl --output text 2>/dev/null || echo "unknown")

    echo ""
    echo "=== AWS Deployment Summary ==="
    echo "  Server IP:  $SERVER_IP"
    echo "  Lambda URL: $LAMBDA_URL"
}

mode_stop() {
    if [ ! -f "$PIDFILE" ]; then
        echo "No pidfile found at $PIDFILE — nothing to stop."
        exit 0
    fi

    read -r SERVER_PID LAMBDA_PID < "$PIDFILE"

    echo "Stopping processes..."
    for pid in $SERVER_PID $LAMBDA_PID; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            echo "  Killed PID $pid"
        else
            echo "  PID $pid already stopped"
        fi
    done

    rm -f "$PIDFILE"
    echo "Done."
}

# Main dispatch
case "${1:-}" in
    local)
        mode_local
        ;;
    aws)
        shift
        mode_aws "$@"
        ;;
    stop)
        mode_stop
        ;;
    *)
        usage
        ;;
esac
