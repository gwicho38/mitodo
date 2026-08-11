{
  description = "a TUI todo tracker over plain markdown checklists";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    {
      overlays.default = final: prev: {
        mitodo = final.callPackage ./nix/package.nix { };
      };
      
      homeManager.default = import ./nix/home-manager-module.nix;
      homeManager.mitodo = self.outputs.homeManager.default;
    }
    // flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };
      in
      {
        packages = {
          mitodo = pkgs.callPackage ./nix/package.nix { };
          default = self.outputs.packages.${system}.mitodo;
        };

        devShells.default = import ./nix/shell.nix { inherit pkgs; };
      }
    );
}
