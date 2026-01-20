{ config, pkgs, censorlessPackage, ... }:

{
  imports = [
    ./module.nix
  ];

  # Basic system configuration
  system.stateVersion = "25.05";

  # Automatically grow root partition on boot
  boot.growPartition = true;

  # Enable SSH
  services.openssh = {
    enable = true;
    settings = {
      PermitRootLogin = "prohibit-password";
      PasswordAuthentication = false;
    };
  };

  # Configure censorless-server
  services.censorless-server = {
    enable = true;
    package = censorlessPackage;
    privateKey = "f0f8e8130b59e49319c4d175ae0fd5b5fbc456360d4bc0fd3708f830bb86c312";
    port = 1337;
    listenAddress = "0.0.0.0";
    verbosity = "debug";
  };

  # Basic networking
  networking.firewall.enable = true;

  # Set your SSH public key
  users.users.root.openssh.authorizedKeys.keys = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF8pQPygwDOXzhe0qS2SU7ByYbLZ9dY9GW0TXacKr2Tz"
  ];

  # Basic packages
  environment.systemPackages = with pkgs; [
    vim
    htop
    curl
  ];
}
