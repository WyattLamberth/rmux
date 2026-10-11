{
  description = "rmux: safe terminal multiplexer with deterministic checks";

  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
      manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      # Exclude local build output, logs, and telemetry data from the derivation.
      src = nixpkgs.lib.fileset.toSource {
        root = ./.;
        fileset = nixpkgs.lib.fileset.unions [
          ./Cargo.toml ./Cargo.lock ./src ./crates ./tests ./examples ./scripts ./config
        ];
      };
      packageFor = system:
        let pkgs = pkgsFor system;
        in pkgs.rustPlatform.buildRustPackage {
          pname = manifest.package.name;
          version = manifest.package.version;
          inherit src;
          cargoLock.lockFile = ./Cargo.lock;
          strictDeps = true;
          cargoBuildFlags = [ "--workspace" "--locked" "--bin" "rmux" ];
          cargoTestFlags = [ "--workspace" "--locked" "--all-features" ];
          nativeCheckInputs = [ pkgs.coreutils ];
          doCheck = true;
          RMUX_TEST_SEED = "42";
          RMUX_TEST_STEPS = "2000";
          RMUX_TEST_SHELL = pkgs.runtimeShell;
          # Network and real-PTY integration tests have their own GitHub gate.
          # Keep sandboxed package checks deterministic and independent of /dev/pts.
          checkPhase = ''
            runHook preCheck
            cargo test --offline --workspace --locked --all-features --lib --bins
            cargo test --offline --workspace --locked --all-features --test universe
            cargo test --offline --workspace --locked --all-features --doc
            runHook postCheck
          '';
          postInstall = ''
            test -x "$out/bin/rmux"
          '';
          meta = {
            description = manifest.package.description;
            license = pkgs.lib.licenses.mit;
            mainProgram = "rmux";
            platforms = systems;
          };
        };
    in {
      packages = forAllSystems (system: {
        default = packageFor system;
        rmux = self.packages.${system}.default;
      });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/rmux";
        };
      });

      checks = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          package = self.packages.${system}.default;
        in {
          package = package;
          format = pkgs.runCommand "rmux-format" {
            nativeBuildInputs = [ pkgs.cargo pkgs.rustfmt ];
          } ''
            cp -r ${src} source
            chmod -R u+w source
            cd source
            export CARGO_HOME=$TMPDIR/rmux-cargo
            cargo fmt --all -- --check
            touch $out
          '';
          clippy = package.overrideAttrs (old: {
            nativeBuildInputs = (old.nativeBuildInputs or []) ++ [ pkgs.clippy ];
            checkPhase = ''
              runHook preCheck
              cargo clippy --offline --workspace --locked --all-targets --all-features -- -D warnings
              runHook postCheck
            '';
          });
          universe = package.overrideAttrs (_: {
            checkPhase = ''
              runHook preCheck
              CARGO_NET_OFFLINE=true bash scripts/test-seeds.sh 0 1 42 1337 18446744073709551615
              runHook postCheck
            '';
          });
        });

      devShells = forAllSystems (system:
        let pkgs = pkgsFor system;
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo rustc rustfmt clippy rust-analyzer
              python3 git pkg-config actionlint shellcheck nixfmt cargo-audit
            ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            RMUX_TEST_SHELL = pkgs.runtimeShell;
          };
        });
    };
}
