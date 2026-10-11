{
  description = "rmux attached terminal multiplexer";
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forSystems = nixpkgs.lib.genAttrs systems;
      packageFor = system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.rustPlatform.buildRustPackage {
          pname = "rmux";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          # Real PTY/outer-terminal integration tests require /dev/ptmx.
          # Run them with cargo test outside the Nix sandbox; unit tests run here.
          cargoTestFlags = [ "--lib" ];
          meta = { description = "Attached terminal multiplexer"; mainProgram = "rmux"; platforms = systems; };
        };
    in {
      packages = forSystems (system: { default = packageFor system; });
      checks = forSystems (system: { package = packageFor system; });
      devShells = forSystems (system:
        let pkgs = import nixpkgs { inherit system; };
        in { default = pkgs.mkShell { packages = [ pkgs.cargo pkgs.rustc pkgs.clippy pkgs.rustfmt ]; }; });
    };
}
