#!/usr/bin/env bash
# Stress test censorless-ng SOCKS proxy with multiple clients and concurrent traffic
#
# Usage:
#   ./stress-test.sh [OPTIONS]
#     --mode local|aws       (default: local)
#     --clients N            Number of client instances (default: 2)
#     --concurrency N        Parallel curls per client (default: 10)
#     --requests N           Total requests per client (default: 50)
#     --urls-file FILE       URL list file (default: stress-urls.txt)
#     --no-fail-fast         Don't abort on first error

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Defaults
MODE="local"
NUM_CLIENTS=2
CONCURRENCY=10
REQUESTS=50
URLS_FILE="stress-urls.txt"
FAIL_FAST=1

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --mode)       MODE="$2"; shift 2 ;;
        --clients)    NUM_CLIENTS="$2"; shift 2 ;;
        --concurrency) CONCURRENCY="$2"; shift 2 ;;
        --requests)   REQUESTS="$2"; shift 2 ;;
        --urls-file)  URLS_FILE="$2"; shift 2 ;;
        --no-fail-fast) FAIL_FAST=0; shift ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo "  --mode local|aws       (default: local)"
            echo "  --clients N            Number of client instances (default: 2)"
            echo "  --concurrency N        Parallel curls per client (default: 10)"
            echo "  --requests N           Total requests per client (default: 50)"
            echo "  --urls-file FILE       URL list file (default: stress-urls.txt)"
            echo "  --no-fail-fast         Don't abort on first error"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [ ! -f "$URLS_FILE" ]; then
    echo "Error: URLs file not found: $URLS_FILE"
    exit 1
fi

# Read URLs into array
mapfile -t URLS < "$URLS_FILE"
if [ ${#URLS[@]} -eq 0 ]; then
    echo "Error: No URLs found in $URLS_FILE"
    exit 1
fi

# Temp dir for configs and logs
TMPDIR=$(mktemp -d "/tmp/censorless-stress.XXXXXX")
CLIENT_PIDS=()
FAIL_FLAG="$TMPDIR/.failed"

cleanup() {
    echo ""
    echo "=== Cleaning up ==="
    for pid in "${CLIENT_PIDS[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            echo "  Killed client PID $pid"
        fi
    done
    rm -rf "$TMPDIR"
    echo "  Removed temp dir $TMPDIR"
}
trap cleanup EXIT

# Determine server/lambda settings based on mode
if [ "$MODE" = "local" ]; then
    LAMBDA_URL="http://localhost:9000/lambda-url/lambda"
    SERVER_HOST="localhost"
    SERVER_PORT=1337
    SERVER_PUBKEY="3fed554a9f79975656af858eaceca5a7ab5f8e34ba9323a550f01c9f843a6d80"
elif [ "$MODE" = "aws" ]; then
    # Try tofu first, fall back to client-aws.toml
    SERVER_HOST=$(tofu output -raw server_ip 2>/dev/null || true)
    if [ -z "$SERVER_HOST" ] && [ -f "client-aws.toml" ]; then
        SERVER_HOST=$(grep '^host' client-aws.toml | head -1 | sed 's/.*= *"\(.*\)"/\1/')
    fi
    if [ -z "$SERVER_HOST" ]; then
        echo "Error: Cannot determine server IP. Run deploy first or ensure client-aws.toml exists."
        exit 1
    fi

    # Lambda URL from client-aws.toml or AWS CLI
    LAMBDA_URL=$(grep '^lambda ' client-aws.toml 2>/dev/null | sed 's/.*= *"\(.*\)"/\1/' || true)
    if [ -z "$LAMBDA_URL" ]; then
        LAMBDA_URL=$(aws lambda get-function-url-config \
            --function-name censorless-v2-lambda \
            --query FunctionUrl --output text 2>/dev/null || true)
    fi
    if [ -z "$LAMBDA_URL" ]; then
        echo "Error: Cannot determine lambda URL."
        exit 1
    fi

    SERVER_PORT=1337
    SERVER_PUBKEY=$(grep '^public_key' client-aws.toml 2>/dev/null | head -1 | sed 's/.*= *"\(.*\)"/\1/' || true)
    if [ -z "$SERVER_PUBKEY" ]; then
        SERVER_PUBKEY="3fed554a9f79975656af858eaceca5a7ab5f8e34ba9323a550f01c9f843a6d80"
    fi
else
    echo "Error: Unknown mode '$MODE'. Use 'local' or 'aws'."
    exit 1
fi

echo "=== Stress Test Configuration ==="
echo "  Mode:        $MODE"
echo "  Clients:     $NUM_CLIENTS"
echo "  Concurrency: $CONCURRENCY per client"
echo "  Requests:    $REQUESTS per client"
echo "  URLs:        ${#URLS[@]} URLs from $URLS_FILE"
echo "  Lambda:      $LAMBDA_URL"
echo "  Server:      $SERVER_HOST:$SERVER_PORT"
echo "  Fail-fast:   $([ "$FAIL_FAST" = 1 ] && echo yes || echo no)"
echo ""

# Generate keypairs and client configs
echo "=== Generating client configs ==="
for i in $(seq 0 $((NUM_CLIENTS - 1))); do
    # Generate Ed25519 keypair
    openssl genpkey -algorithm ED25519 -out "$TMPDIR/key-$i.pem" 2>/dev/null
    PRIV_KEY=$(openssl pkey -in "$TMPDIR/key-$i.pem" -outform DER 2>/dev/null | tail -c 32 | xxd -p -c 64)

    PORT=$((1080 + i))

    cat > "$TMPDIR/client-$i.toml" <<EOF
private_key = "$PRIV_KEY"
lambda = "$LAMBDA_URL"
lambda_buffer = 1048576
timeout = 200
idle_timeout = 300000

[[servers]]
public_key = "$SERVER_PUBKEY"
host = "$SERVER_HOST"
port = $SERVER_PORT
EOF

    echo "  Client $i: port $PORT, config $TMPDIR/client-$i.toml"
done

# Find client binary: prefer CLIENT_BIN env, then PATH, then cargo target dirs
if [ -z "$CLIENT_BIN" ]; then
    CLIENT_BIN=$(command -v censorless 2>/dev/null || true)
fi
if [ -z "$CLIENT_BIN" ] || [ ! -x "$CLIENT_BIN" ]; then
    CLIENT_BIN="$SCRIPT_DIR/target/release/censorless"
fi
if [ ! -x "$CLIENT_BIN" ]; then
    CLIENT_BIN="$SCRIPT_DIR/target/debug/censorless"
fi
if [ ! -x "$CLIENT_BIN" ]; then
    echo "Error: Client binary not found."
    echo "  Either run 'nix run .#stress-test', set CLIENT_BIN, or 'cargo build -p client'."
    exit 1
fi
echo "  Client binary: $CLIENT_BIN"

# Start client instances
echo ""
echo "=== Starting $NUM_CLIENTS client instances ==="
for i in $(seq 0 $((NUM_CLIENTS - 1))); do
    PORT=$((1080 + i))
    "$CLIENT_BIN" --config "$TMPDIR/client-$i.toml" --bind "127.0.0.1:$PORT" &
    CLIENT_PIDS+=($!)
    echo "  Client $i: PID ${CLIENT_PIDS[$i]} on port $PORT"
done

# Wait for SOCKS ports
echo ""
echo "=== Waiting for SOCKS ports ==="
for i in $(seq 0 $((NUM_CLIENTS - 1))); do
    PORT=$((1080 + i))
    attempts=0
    while [ $attempts -lt 20 ]; do
        if nc -z 127.0.0.1 "$PORT" 2>/dev/null; then
            echo "  Port $PORT ready"
            break
        fi
        attempts=$((attempts + 1))
        sleep 0.5
    done
    if [ $attempts -eq 20 ]; then
        echo "  ERROR: Port $PORT not ready after 10s"
        exit 1
    fi
done

# Run stress traffic
echo ""
echo "=== Running stress traffic ==="
echo "  $NUM_CLIENTS clients x $CONCURRENCY concurrency x $REQUESTS requests"
echo ""

CURL_PIDS=()

for i in $(seq 0 $((NUM_CLIENTS - 1))); do
    PORT=$((1080 + i))
    LOGFILE="$TMPDIR/results-$i.log"

    # Spawn $CONCURRENCY workers for this client
    requests_per_worker=$((REQUESTS / CONCURRENCY))
    remainder=$((REQUESTS % CONCURRENCY))

    for w in $(seq 0 $((CONCURRENCY - 1))); do
        # Distribute remainder across first workers
        worker_requests=$requests_per_worker
        if [ "$w" -lt "$remainder" ]; then
            worker_requests=$((worker_requests + 1))
        fi

        (
            url_idx=0
            for _r in $(seq 1 "$worker_requests"); do
                # Check fail-fast flag
                if [ "$FAIL_FAST" = 1 ] && [ -f "$FAIL_FLAG" ]; then
                    exit 1
                fi

                url="${URLS[$((url_idx % ${#URLS[@]}))]}"
                url_idx=$((url_idx + 1))

                result=$(curl -x "socks5h://127.0.0.1:$PORT" \
                    -H "Connection: close" \
                    --connect-timeout 15 --max-time 30 \
                    -s -o /dev/null \
                    -w '%{http_code} %{time_total}' \
                    "$url" 2>/dev/null) || result="000 0.000000"

                echo "$result" >> "$LOGFILE"
                http_code="${result%% *}"

                if [ "$http_code" = "000" ] || [ "$http_code" -ge 500 ] 2>/dev/null; then
                    echo "  FAIL [client $i worker $w]: $http_code on $url" >&2
                    if [ "$FAIL_FAST" = 1 ]; then
                        touch "$FAIL_FLAG"
                        exit 1
                    fi
                fi
            done
        ) &
        CURL_PIDS+=($!)
    done
done

echo "  Spawned ${#CURL_PIDS[@]} curl workers, waiting for completion..."

# Wait for all curl processes, track if any failed
any_failed=0
for pid in "${CURL_PIDS[@]}"; do
    if ! wait "$pid" 2>/dev/null; then
        any_failed=1
    fi
done

if [ -f "$FAIL_FLAG" ]; then
    echo ""
    echo "  ABORTED: A request failed (fail-fast enabled)"
fi

echo "  All workers finished."

# Generate report
echo ""
echo "==========================================="
echo "            STRESS TEST RESULTS            "
echo "==========================================="
echo ""

total_requests=0
total_success=0
total_fail=0

for i in $(seq 0 $((NUM_CLIENTS - 1))); do
    LOGFILE="$TMPDIR/results-$i.log"
    PORT=$((1080 + i))

    if [ ! -f "$LOGFILE" ]; then
        echo "Client $i (port $PORT): No results"
        continue
    fi

    c_total=$(wc -l < "$LOGFILE")
    c_success=$(awk '$1 >= 200 && $1 < 300 { count++ } END { print count+0 }' "$LOGFILE")
    c_fail=$((c_total - c_success))

    # Compute latency stats (only for successful requests)
    latencies=$(awk '$1 >= 200 && $1 < 300 { print $2 }' "$LOGFILE" | sort -n)

    if [ -n "$latencies" ]; then
        count=$(echo "$latencies" | wc -l)
        avg=$(echo "$latencies" | awk '{ sum += $1 } END { printf "%.3f", sum/NR }')
        p50_idx=$(( (count * 50 + 99) / 100 ))
        p95_idx=$(( (count * 95 + 99) / 100 ))
        p50=$(echo "$latencies" | sed -n "${p50_idx}p")
        p95=$(echo "$latencies" | sed -n "${p95_idx}p")
    else
        avg="N/A"
        p50="N/A"
        p95="N/A"
    fi

    printf "Client %d (port %d):\n" "$i" "$PORT"
    printf "  Total: %-6d  Success: %-6d  Failed: %-6d\n" "$c_total" "$c_success" "$c_fail"
    printf "  Latency — avg: %ss  p50: %ss  p95: %ss\n" "$avg" "$p50" "$p95"
    echo ""

    total_requests=$((total_requests + c_total))
    total_success=$((total_success + c_success))
    total_fail=$((total_fail + c_fail))
done

# Overall summary
echo "-------------------------------------------"
echo "OVERALL:"
printf "  Total: %-6d  Success: %-6d  Failed: %-6d\n" "$total_requests" "$total_success" "$total_fail"

# Overall latency from all logs
all_latencies=$(for i in $(seq 0 $((NUM_CLIENTS - 1))); do
    awk '$1 >= 200 && $1 < 300 { print $2 }' "$TMPDIR/results-$i.log" 2>/dev/null
done | sort -n)

if [ -n "$all_latencies" ]; then
    count=$(echo "$all_latencies" | wc -l)
    avg=$(echo "$all_latencies" | awk '{ sum += $1 } END { printf "%.3f", sum/NR }')
    p50_idx=$(( (count * 50 + 99) / 100 ))
    p95_idx=$(( (count * 95 + 99) / 100 ))
    p50=$(echo "$all_latencies" | sed -n "${p50_idx}p")
    p95=$(echo "$all_latencies" | sed -n "${p95_idx}p")
    printf "  Latency — avg: %ss  p50: %ss  p95: %ss\n" "$avg" "$p50" "$p95"
fi

if [ "$total_requests" -gt 0 ]; then
    success_rate=$((total_success * 100 / total_requests))
    printf "  Success rate: %d%%\n" "$success_rate"
fi

echo "==========================================="

# Exit with error if any requests failed
if [ "$any_failed" = 1 ] || [ "$total_fail" -gt 0 ]; then
    exit 1
fi
