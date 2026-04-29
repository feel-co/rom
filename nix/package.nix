{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "rom";
  version = "0.1.0";

  src = let
    fs = lib.fileset;
    s = ../.;
  in
    fs.toSource {
      root = s;
      fileset = fs.unions [
        (s + /crates)
        (s + /rom)
        (s + /Cargo.lock)
        (s + /Cargo.toml)
      ];
    };

  cargoLock.lockFile = ../Cargo.lock;
  enableParallelBuildingByDefault = true;

  meta = {
    description = "Pretty build graphs for your pretty Nix builds";
    maintainers = with lib.maintainers; [NotAShelf];
    license = lib.licenses.eupl12;
  };
})
