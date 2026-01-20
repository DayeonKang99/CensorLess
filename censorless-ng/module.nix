{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.censorless-server;

  configFile = pkgs.writeText "server.toml" ''
    private_key = "${cfg.privateKey}"
    port = ${toString cfg.port}
    addr = "${cfg.listenAddress}"
    buffer_max = ${toString cfg.bufferMax}
    timeout = ${toString cfg.timeout}
    idle_timeout = ${toString cfg.idleTimeout}
    connections_per_pkey = ${toString cfg.connectionsPerPkey}
    allow_private = ${if cfg.allowPrivate then "true" else "false"}
  '';
in
{
  options.services.censorless-server = {
    enable = mkEnableOption "censorless-ng server";

    package = mkOption {
      type = types.package;
      description = "The censorless package to use";
    };

    privateKey = mkOption {
      type = types.str;
      description = "Hex-encoded Ed25519 private key for the server";
    };

    port = mkOption {
      type = types.port;
      default = 1337;
      description = "Port to listen on";
    };

    listenAddress = mkOption {
      type = types.str;
      default = "0.0.0.0";
      description = "Address to bind to";
    };

    bufferMax = mkOption {
      type = types.int;
      default = 1048576;
      description = "Maximum buffer size in bytes";
    };

    timeout = mkOption {
      type = types.int;
      default = 5000;
      description = "Timeout in milliseconds";
    };

    idleTimeout = mkOption {
      type = types.int;
      default = 300000;
      description = "Idle timeout in milliseconds";
    };

    connectionsPerPkey = mkOption {
      type = types.int;
      default = 100;
      description = "Maximum connections per client public key";
    };

    allowPrivate = mkOption {
      type = types.bool;
      default = false;
      description = "Allow connections to private IP addresses";
    };

    verbosity = mkOption {
      type = types.str;
      default = "info";
      description = "Log verbosity level (error, warn, info, debug, trace)";
    };
  };

  config = mkIf cfg.enable {
    systemd.services.censorless-server = {
      description = "Censorless-ng proxy server";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/censorless-server --config ${configFile} --verbosity ${cfg.verbosity}";
        Restart = "always";
        RestartSec = "10s";
        # Hardening
        DynamicUser = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        NoNewPrivileges = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" ];
        RestrictNamespaces = true;
        LockPersonality = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        MemoryDenyWriteExecute = true;
      };
    };

    networking.firewall.allowedTCPPorts = [ cfg.port ];
  };
}
