{
  description = "muman (Music Manager)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      overlays.default = final: _prev: {
        muman = final.callPackage ./nix/package.nix { };
      };

      packages = forAllSystems (pkgs: {
        muman = pkgs.callPackage ./nix/package.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.muman;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.muman ];
          packages = with pkgs; [
            clippy
            rustfmt
            rust-analyzer
            cargo-deny
            ffmpeg-headless
            yt-dlp
          ];
        };
      });

      checks = forAllSystems (pkgs: {
        muman = self.packages.${pkgs.stdenv.hostPlatform.system}.muman;
        nixfmt = pkgs.runCommand "nixfmt-check" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
          nixfmt --check ${./flake.nix} ${./default.nix} ${./nix/package.nix}
          touch $out
        '';
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
