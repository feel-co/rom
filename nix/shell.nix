{
  mkShell,
  rustc,
  cargo,
  rustfmt,
  clippy,
  taplo,
  rust-analyzer-unwrapped,
  rustPlatform,
  cargo-nextest,
}:
mkShell {
  name = "rust";

  packages = [
    rustc
    cargo

    (rustfmt.override {asNightly = true;})
    clippy
    taplo
    rust-analyzer-unwrapped

    cargo-nextest
  ];

  env.RUST_SRC_PATH = "${rustPlatform.rustLibSrc}";
}
