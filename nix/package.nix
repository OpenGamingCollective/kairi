{ inputs, ... }:
{
  perSystem =
    {
      system,
      pkgs,
      lib,
      ...
    }:
    let
      fenixpkgs = inputs.fenix.packages.${system}.fromToolchainFile {
        file = ../rust-toolchain.toml;
        sha256 = "sha256-p8h3Sl/YRByZfZTAKXdsvF6xEenXKrXSVvpphmZENH4=";
      };
      rustPlatform = pkgs.makeRustPlatform {
        cargo = fenixpkgs;
        rustc = fenixpkgs;
      };
      kairi = rustPlatform.buildRustPackage {
        pname = "kairi";
        version = "0.1.0";

        __structuredAttrs = true;

        src = ./..;
        cargoLock = {
          lockFile = ../Cargo.lock;
          outputHashes = {
            # Not in crate.io yet
            "typesafe-api-0.1.0" = "sha256-9oXF0eThMMayLVqBQ60ryR+j9c/OFxd89Ru5FV+YQUQ=";
          };
        };
        meta = {
          description = "Kairi bot";
          license = lib.licenses.gpl3Only;
          mainProgram = "kairi";
          platforms = lib.platforms.unix;
        };
      };
    in
    {
      packages.default = kairi;
      packages.kairi = kairi;
    };
}
