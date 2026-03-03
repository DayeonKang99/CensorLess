#!/usr/bin/env python3
"""
Selenium test: start the censorless proxy and open a page through it.

Usage:
  python selenium-test.py [--mode local|aws] [--headless] [--url URL]

Modes:
  local  - uses client.toml  (expects local lambda + server running)
  aws    - uses client-aws.toml  (routes through deployed AWS infra)

The script starts the censorless client binary, waits for the SOCKS port,
opens the URL in Firefox via the proxy, then tears everything down.
"""

import argparse
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))


def find_client_binary():
    """Find the censorless client binary."""
    # 1. CENSORLESS_BIN env override
    env_bin = os.environ.get("CENSORLESS_BIN")
    if env_bin and os.access(env_bin, os.X_OK):
        return env_bin

    # 2. On PATH (works inside nix run / nix develop)
    on_path = shutil.which("censorless")
    if on_path:
        return on_path

    # 3. Cargo target dirs
    for profile in ("release", "debug"):
        candidate = os.path.join(SCRIPT_DIR, "target", profile, "censorless")
        if os.access(candidate, os.X_OK):
            return candidate

    return None


def wait_for_port(host, port, timeout=15):
    """Wait until a TCP port is accepting connections."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1):
                return True
        except OSError:
            time.sleep(0.25)
    return False


def main():
    parser = argparse.ArgumentParser(
        description="Start censorless proxy and open a page in Firefox"
    )
    parser.add_argument(
        "--mode",
        choices=["local", "aws"],
        default="local",
        help="Which config to use (default: local)",
    )
    parser.add_argument("--headless", action="store_true", help="Run Firefox headless")
    parser.add_argument(
        "--timeout",
        type=int,
        default=60,
        help="Page load timeout in seconds (default: 60)",
    )
    parser.add_argument(
        "--url",
        default="https://www.cnn.com",
        help="URL to open (default: https://www.cnn.com)",
    )
    parser.add_argument(
        "--bind",
        default="127.0.0.1:1080",
        help="SOCKS bind address for the client (default: 127.0.0.1:1080)",
    )
    parser.add_argument(
        "--config",
        help="Explicit path to client config (overrides --mode)",
    )
    parser.add_argument(
        "--keep-open",
        type=int,
        metavar="SECONDS",
        default=0,
        help="Keep the browser open for N seconds after loading (default: 0)",
    )
    args = parser.parse_args()

    # --- Resolve client config ---
    if args.config:
        config_path = args.config
    elif args.mode == "aws":
        config_path = os.path.join(SCRIPT_DIR, "client-aws.toml")
    else:
        config_path = os.path.join(SCRIPT_DIR, "client.toml")

    if not os.path.isfile(config_path):
        print(f"Error: config file not found: {config_path}")
        sys.exit(1)

    # --- Find client binary ---
    client_bin = find_client_binary()
    if not client_bin:
        print(
            "Error: censorless binary not found.\n"
            "  Run inside 'nix develop', use 'nix run .#selenium-test',\n"
            "  set CENSORLESS_BIN, or 'cargo build -p client'."
        )
        sys.exit(1)

    proxy_host, proxy_port_str = args.bind.rsplit(":", 1)
    proxy_port = int(proxy_port_str)

    print(f"=== Selenium Proxy Test ===")
    print(f"  Mode:    {args.mode}")
    print(f"  Config:  {config_path}")
    print(f"  Binary:  {client_bin}")
    print(f"  Bind:    {args.bind}")
    print(f"  URL:     {args.url}")
    print(f"  Headless: {args.headless}")
    print()

    # --- Start the censorless client ---
    client_log = tempfile.NamedTemporaryFile(
        prefix="censorless-client-", suffix=".log", dir="/tmp", delete=False
    )
    print(f"Starting censorless client (log: {client_log.name}) ...")
    client_proc = subprocess.Popen(
        [client_bin, "--config", config_path, "--bind", args.bind],
        stdout=client_log,
        stderr=subprocess.STDOUT,
        preexec_fn=os.setsid,  # own process group for clean teardown
    )

    driver = None
    profile_dir = None

    try:
        # Wait for the SOCKS port
        print(f"Waiting for SOCKS port {args.bind} ...")
        if not wait_for_port(proxy_host, proxy_port, timeout=15):
            print(f"Error: SOCKS port {args.bind} not ready after 15s")
            print(f"  Check client log: {client_log.name}")
            sys.exit(1)
        print("  SOCKS port ready")

        # --- Set up Firefox with isolated profile ---
        profile_dir = tempfile.mkdtemp(prefix="firefox-selenium-", dir="/tmp")
        print(f"Using temporary Firefox profile: {profile_dir}")

        options = Options()
        options.add_argument("-profile")
        options.add_argument(profile_dir)

        if args.headless:
            options.add_argument("--headless")

        # SOCKS5 proxy with remote DNS
        options.set_preference("network.proxy.type", 1)
        options.set_preference("network.proxy.socks", proxy_host)
        options.set_preference("network.proxy.socks_port", proxy_port)
        options.set_preference("network.proxy.socks_version", 5)
        options.set_preference("network.proxy.socks_remote_dns", True)

        # Suppress first-run noise
        options.set_preference("browser.shell.checkDefaultBrowser", False)
        options.set_preference(
            "browser.startup.homepage_override.mstone", "ignore"
        )
        options.set_preference(
            "datareporting.policy.dataSubmissionEnabled", False
        )
        options.set_preference(
            "toolkit.telemetry.reportingpolicy.firstRun", False
        )

        service = Service(log_output="/tmp/geckodriver.log")

        print("Launching Firefox ...")
        driver = webdriver.Firefox(options=options, service=service)
        driver.set_page_load_timeout(args.timeout)

        print(f"Navigating to {args.url} ...")
        start = time.monotonic()
        driver.get(args.url)
        elapsed = time.monotonic() - start
        print(f"Page loaded in {elapsed:.2f}s")
        print(f"Title: {driver.title}")

        if driver.title:
            print("SUCCESS: page loaded with a title")
        else:
            print("WARNING: page loaded but title is empty")
            sys.exit(1)

        if args.keep_open > 0:
            print(f"Keeping browser open for {args.keep_open}s ...")
            time.sleep(args.keep_open)

    except Exception as e:
        print(f"FAILED: {e}")
        sys.exit(1)

    finally:
        # --- Teardown ---
        print()
        print("=== Cleaning up ===")

        if driver:
            try:
                driver.quit()
                print("  Firefox stopped")
            except Exception:
                pass

        # Kill the censorless client (whole process group)
        if client_proc.poll() is None:
            os.killpg(os.getpgid(client_proc.pid), signal.SIGTERM)
            try:
                client_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(os.getpgid(client_proc.pid), signal.SIGKILL)
            print(f"  Censorless client stopped (PID {client_proc.pid})")
        else:
            print(
                f"  Censorless client already exited (code {client_proc.returncode})"
            )

        print(f"  Client log: {client_log.name}")
        if profile_dir:
            print(f"  Firefox profile: {profile_dir}")


if __name__ == "__main__":
    main()
