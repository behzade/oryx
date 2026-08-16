{
  description = "Oryx development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          lib = pkgs.lib;
          rustConfig = builtins.fromTOML (builtins.readFile ./rust-toolchain.toml);
          rustToolchain = pkgs.rust-bin.fromRustupToolchain {
            channel = rustConfig.toolchain.channel;
            profile = "minimal";
            components = [
              "clippy"
              "rust-src"
              "rustfmt"
            ];
          };
          cargoPackager = pkgs.rustPlatform.buildRustPackage rec {
            pname = "cargo-packager";
            version = "0.11.8";

            src = pkgs.fetchCrate {
              inherit pname version;
              hash = "sha256-DjqrsomwtM5JzGrBIjfREZ15pUijza+/p+3CwXe+dSY=";
            };

            cargoHash = "sha256-rSNBn8CkqJN52ApHjhH6wJpy23DLv5BSN/rjWZrl5mk=";
            doCheck = false;
          };
          appleXcrun = pkgs.writeShellScriptBin "xcrun" ''
            unset DEVELOPER_DIR SDKROOT
            exec /usr/bin/xcrun "$@"
          '';
          linuxBuildInputs = lib.optionals pkgs.stdenv.isLinux [
            pkgs.alsa-lib
            pkgs.dbus
            pkgs.fontconfig
            pkgs.libGL
            pkgs.libX11
            pkgs.libXcursor
            pkgs.libXi
            pkgs.libXrandr
            pkgs.libxcb
            pkgs.libxkbcommon
            pkgs.vulkan-loader
            pkgs.wayland
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [
              rustToolchain
              cargoPackager
              pkgs.cmake
              pkgs.ffmpeg-headless
              pkgs.git
              pkgs.gnumake
              pkgs.perl
              pkgs.pkg-config
              pkgs.rust-analyzer
              pkgs.yt-dlp
            ];

            buildInputs = linuxBuildInputs;

            LD_LIBRARY_PATH = lib.makeLibraryPath linuxBuildInputs;

            shellHook = ''
              ${lib.optionalString pkgs.stdenv.isDarwin ''
                export PATH="${appleXcrun}/bin:$PATH"
              ''}
              echo "Oryx dev shell: $(rustc --version)"
              echo "Run 'make help' to list common commands."
            '';
          };
        }
      );
    };
}
