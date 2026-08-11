{ pkgs }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    cargo
    rustc
    rust-analyzer
    clippy
    rustfmt
  ];
}
