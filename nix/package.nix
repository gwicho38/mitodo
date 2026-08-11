{ lib
, rustPlatform
}:

rustPlatform.buildRustPackage {
  pname = "mitodo";
  version = (lib.importTOML ../Cargo.toml).package.version;

  src = ../.;

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  meta = with lib; {
    description = "a TUI todo tracker over plain markdown checklists";
    homepage = "https://github.com/gwicho38/mitodo";
    license = licenses.gpl3Plus;
    maintainers = [ "gwicho38" ];
    mainProgram = "mitodo";
  };
}
