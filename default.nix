# muman for `nix-build` without flakes, built with the nixpkgs that
# flake.lock pins.
{
  pkgs ?
    let
      locked = (builtins.fromJSON (builtins.readFile ./flake.lock)).nodes.nixpkgs.locked;
    in
    import (fetchTarball {
      url = "https://github.com/${locked.owner}/${locked.repo}/archive/${locked.rev}.tar.gz";
      sha256 = locked.narHash;
    }) { },
}:

pkgs.callPackage ./nix/package.nix { }
