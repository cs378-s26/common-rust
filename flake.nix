{
  description = "Rust kernel";

  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
        {
          devShells.default = with pkgs; mkShell {
            buildInputs = [
              llvmPackages_21.clang-unwrapped
              llvmPackages_21.bintools
              rustToolchain
              pkg-config
              openssl
              qemu
              wabt
              gdb
              e2fsprogs
              sccache
            ];
            LIBCLANG_PATH = pkgs.lib.makeLibraryPath [ pkgs.llvmPackages_21.libclang.lib ];
            RUSTC_WRAPPER = "sccache";
          };
        }
    );
}
