{ inputs, ... }:
{
  perSystem =
    { pkgs, system, ... }:
    let
      fenixpkgs = inputs.fenix.packages.${system}.fromToolchainFile {
        file = ../rust-toolchain.toml;
        sha256 = "sha256-p8h3Sl/YRByZfZTAKXdsvF6xEenXKrXSVvpphmZENH4=";
      };
    in
    {
      devShells.default = pkgs.mkShell {
        buildInputs = [
          fenixpkgs
          pkgs.just
        ];
      };
    };
}
