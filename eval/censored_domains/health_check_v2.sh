#!/bin/bash

# Health Check Script with Proxy Support and CSV Output
# Usage: ./health_check.sh <domains_file> [proxy_url] [output_csv]

# Check if domains file is provided
if [ $# -lt 1 ]; then
    echo "Usage: $0 <domains_file> [proxy_url] [output_csv]"
    echo "Example: $0 domains.txt http://proxy.example.com:8080 results.csv"
    exit 1
fi

DOMAINS_FILE="$1"
PROXY_URL="${2:-}"  # Optional proxy URL
OUTPUT_CSV="${3:-health_check_results.csv}"  # Default output file

# Check if domains file exists
if [ ! -f "$DOMAINS_FILE" ]; then
    echo "Error: Domains file '$DOMAINS_FILE' not found"
    exit 1
fi

# Initialize CSV file with headers
echo "Timestamp,Domain,Attempt,Status,HTTP_Code,Time_Total_Seconds,Time_Connect_Seconds,Error_Message" > "$OUTPUT_CSV"

# Function to perform health check
health_check() {
    local domain="$1"
    local attempt="$2"
    local proxy_arg=""
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    
    # Add proxy if provided
    if [ -n "$PROXY_URL" ]; then
        proxy_arg="--proxy $PROXY_URL"
    fi
    
    echo "Checking: $domain (Attempt $attempt/2) at $timestamp"
    
    # Perform curl request with 100-second timeout
    response=$(curl -s -o /dev/null -w "%{http_code}|%{time_total}|%{time_connect}" \
        --connect-timeout 100 \
        --max-time 100 \
        $proxy_arg \
        "$domain" 2>&1)
    
    local exit_code=$?
    
    if [ $exit_code -eq 0 ]; then
        # Parse response
        IFS='|' read -r http_code time_total time_connect <<< "$response"
        
        # Write to CSV
        echo "$timestamp,\"$domain\",$attempt,SUCCESS,$http_code,$time_total,$time_connect,\"\"" >> "$OUTPUT_CSV"
        
        echo "  ✓ Success - HTTP: $http_code, Total: ${time_total}s"
    else
        # Get error message
        local error_msg="Connection failed (exit code: $exit_code)"
        
        # Try to extract any partial data
        IFS='|' read -r http_code time_total time_connect <<< "$response"
        http_code="${http_code:-000}"
        time_total="${time_total:-0}"
        time_connect="${time_connect:-0}"
        
        # Write to CSV (escape quotes in error message)
        error_msg="${error_msg//\"/\"\"}"
        echo "$timestamp,\"$domain\",$attempt,FAILED,$http_code,$time_total,$time_connect,\"$error_msg\"" >> "$OUTPUT_CSV"
        
        echo "  ✗ Failed - $error_msg"
    fi
}

# Main execution
echo "========================================"
echo "Starting Health Checks"
echo "========================================"
echo "Domains file: $DOMAINS_FILE"
if [ -n "$PROXY_URL" ]; then
    echo "Proxy: $PROXY_URL"
else
    echo "Proxy: None (direct connection)"
fi
echo "Output CSV: $OUTPUT_CSV"
echo "Timeout: 100 seconds"
echo "========================================"
echo ""

# Read domains from file and perform health checks
while IFS= read -r domain || [ -n "$domain" ]; do
    # Skip empty lines and comments
    [[ -z "$domain" || "$domain" =~ ^[[:space:]]*# ]] && continue
    
    # Trim whitespace
    domain=$(echo "$domain" | xargs)
    
    # Skip if empty after trimming
    [ -z "$domain" ] && continue
    
    # Add http:// if no protocol specified
    if [[ ! "$domain" =~ ^https?:// ]]; then
        domain="https://$domain"
    fi
    
    # Perform two health checks for each domain
    health_check "$domain" 1
    sleep 1  # Small delay between attempts
    health_check "$domain" 2
    echo ""
    
done < "$DOMAINS_FILE"

echo "========================================"
echo "Health Checks Completed"
echo "========================================"
echo "Results saved to: $OUTPUT_CSV"
echo ""
echo "Summary:"
total_checks=$(tail -n +2 "$OUTPUT_CSV" | wc -l)
successful_checks=$(tail -n +2 "$OUTPUT_CSV" | grep -c ",SUCCESS,")
failed_checks=$(tail -n +2 "$OUTPUT_CSV" | grep -c ",FAILED,")
echo "  Total checks: $total_checks"
echo "  Successful: $successful_checks"
echo "  Failed: $failed_checks"
