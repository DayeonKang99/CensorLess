{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    deploy-rs = {
      url = "github:serokell/deploy-rs";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane, deploy-rs }:
    let
      cargo_build_targets = {
        "x86_64-linux" = "x86_64-unknown-linux-musl";
        "aarch64-linux" = "aarch64-unknown-linux-musl";
      };

      # Lambda packages for Linux systems
      lambdaPackages = (flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ]
        (system:
          let
            pkgs = (import nixpkgs) {
              inherit system;
              overlays = [ (import rust-overlay) ];

            };
            inherit (pkgs) lib;
            CARGO_BUILD_TARGET = cargo_build_targets.${system};
            rust_toolchain = p: p.rust-bin.stable.latest.default.override {
              targets = [ CARGO_BUILD_TARGET ];
            };
            craneLib = (crane.mkLib pkgs).overrideToolchain rust_toolchain;
          in
          rec {
            packages = {
              censorless-lambda = craneLib.buildPackage {
                src = craneLib.cleanCargoSource ./.;
                strictDeps = true;
                pname = "censorless-lambda";
                cargoExtraArgs = "--workspace --exclude client --exclude server";
                inherit CARGO_BUILD_TARGET;
                CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
              };
              censorless-client-musl = craneLib.buildPackage {
                src = craneLib.cleanCargoSource ./.;
                strictDeps = true;
                pname = "censorless-client-musl";
                cargoExtraArgs = "-p client";
                inherit CARGO_BUILD_TARGET;
                CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
                # Disable glibc's _FORTIFY_SOURCE which replaces memcpy with
                # __memcpy_chk — a symbol musl doesn't provide.
                hardeningDisable = [ "fortify" ];
              };
            };
            checks = {
              inherit (packages) censorless-lambda censorless-client-musl;
            };
          }
        ));

      # Client and server packages for all systems
      clientServerPackages = (flake-utils.lib.eachDefaultSystem (system:
        let
          pkgs = (import nixpkgs) {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          rust_toolchain = p: p.rust-bin.stable.latest.default;
          craneLib = (crane.mkLib pkgs).overrideToolchain rust_toolchain;
        in
        rec {
          packages = {
            censorless = craneLib.buildPackage {
              src = craneLib.cleanCargoSource ./.;
              pname = "censorless";
              cargoExtraArgs = "--workspace --exclude lambda";
            };
            selenium-test =
              let
                python = pkgs.python3.withPackages (ps: [ ps.selenium ]);
              in
              pkgs.writeShellApplication {
                name = "selenium-test";
                runtimeInputs = [
                  python
                  pkgs.geckodriver
                  pkgs.firefox
                  packages.censorless
                ];
                text = ''
                  exec python ${./selenium-test.py} "$@"
                '';
              };
            stress-test = pkgs.writeShellApplication {
              name = "stress-test";
              runtimeInputs = [
                packages.censorless
                pkgs.curl
                pkgs.openssl
                pkgs.xxd
                pkgs.netcat-gnu
                pkgs.coreutils
                pkgs.gawk
                pkgs.gnused
                pkgs.gnugrep
              ];
              text = ''
                export CLIENT_BIN="${packages.censorless}/bin/censorless"
                # Resolve urls file: user-provided flag > CWD file > embedded default
                has_urls_flag=0
                for arg in "$@"; do
                  if [ "$arg" = "--urls-file" ]; then has_urls_flag=1; fi
                done
                if [ "$has_urls_flag" = 0 ]; then
                  if [ -f "stress-urls.txt" ]; then
                    exec bash ${./stress-test.sh} --urls-file "$(pwd)/stress-urls.txt" "$@"
                  else
                    STRESS_URLS_DIR=$(mktemp -d)
                    export STRESS_URLS_DIR
                    cat > "$STRESS_URLS_DIR/stress-urls.txt" <<'URLEOF'
                ${builtins.readFile ./stress-urls.txt}URLEOF
                    exec bash ${./stress-test.sh} --urls-file "$STRESS_URLS_DIR/stress-urls.txt" "$@"
                  fi
                fi
                exec bash ${./stress-test.sh} "$@"
              '';
            };
          };
          apps = {
            censorless = {
              type = "app";
              program = "${packages.censorless}/bin/censorless";
            };
            censorless-server = {
              type = "app";
              program = "${packages.censorless}/bin/censorless-server";
            };
            selenium-test = {
              type = "app";
              program = "${packages.selenium-test}/bin/selenium-test";
            };
            stress-test = {
              type = "app";
              program = "${packages.stress-test}/bin/stress-test";
            };
          };
          checks = {
            inherit (packages) censorless;
          };
          devShells.default = pkgs.mkShell {
            packages = [
              ((rust_toolchain pkgs).override {
                extensions = [ "rust-src" "rustfmt" "rust-analyzer" "clippy" ];
              })
              pkgs.cargo-lambda
              pkgs.awscli
              pkgs.opentofu
              deploy-rs.packages.${system}.default

              # Selenium test dependencies
              (pkgs.python3.withPackages (ps: [ ps.selenium ]))
              pkgs.geckodriver
              pkgs.firefox
            ];

            # Make cross-compilation toolchain available but not default
            shellHook = ''
              export PATH="${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin:$PATH"
            '';
          };
        }));
    in
    # Recursively merge lambda packages with client/server packages
    (nixpkgs.lib.recursiveUpdate lambdaPackages clientServerPackages) // {
      nixosConfigurations.censorless-server = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = [
          "${nixpkgs}/nixos/modules/virtualisation/amazon-image.nix"
          ./server-configuration.nix
          {
            nixpkgs.hostPlatform = "aarch64-linux";

            # Pass the censorless package to the configuration
            _module.args.censorlessPackage = clientServerPackages.packages.aarch64-linux.censorless;
          }
        ];
      };

      deploy.nodes.censorless-server = {
        hostname = ""; # Set this via command line or environment variable
        profiles.system = {
          sshUser = "root";
          path = deploy-rs.lib.aarch64-linux.activate.nixos
            self.nixosConfigurations.censorless-server;
          user = "root";
        };
      };

      # Checks for deploy-rs
      checks = builtins.mapAttrs (system: deployLib: deployLib.deployChecks self.deploy) deploy-rs.lib;
    };
}
