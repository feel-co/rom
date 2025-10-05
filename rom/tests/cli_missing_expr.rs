use std::process::Command;

fn run_rom(args: &[&str]) -> (String, String, i32) {
  let output = Command::new(env!("CARGO_BIN_EXE_rom"))
    .args(args)
    .output()
    .expect("failed to execute rom binary");

  let stdout = String::from_utf8_lossy(&output.stdout).to_string();
  let stderr = String::from_utf8_lossy(&output.stderr).to_string();
  let status = output.status.code().unwrap_or(-1);
  (stdout, stderr, status)
}

#[test]
fn test_parse_args_with_separator_passthrough() {
  // This test verifies the splitting logic for passthrough args
  let args = ["--", "--rebuild", "--refresh"];
  let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
  let (package_and_rom_args, nix_flags) =
    rom::cli::parse_args_with_separator(&args);
  assert!(
    package_and_rom_args.is_empty(),
    "package_and_rom_args should be empty"
  );
  assert_eq!(
    nix_flags,
    vec!["--rebuild", "--refresh"],
    "nix_flags should contain passthrough args"
  );
}

#[test]
fn test_missing_expression_errors() {
  // No expression, no passthrough args
  let (_out, err, status) = run_rom(&["build", "--tree"]);
  assert_ne!(status, 0, "should fail with missing expression");
  assert!(
    err.contains("No package or flake specified for nix build"),
    "should print missing expression error, got: {}",
    err
  );
}

#[test]
fn test_passthrough_args_without_expression_errors() {
  // No expression, only passthrough args after --
  let (_out, err, status) =
    run_rom(&["build", "--tree", "--", "--rebuild", "--refresh"]);
  assert_ne!(
    status, 0,
    "should fail with missing expression even with passthrough args"
  );
  assert!(
    err.contains("No package or flake specified for nix build"),
    "should print missing expression error, got: {}",
    err
  );
}

#[test]
fn test_valid_expression_with_passthrough_args_succeeds() {
  // With expression and passthrough args, should not error about missing
  // expression Use a trivial expression that should always exist (like
  // nixpkgs#hello)
  let (_out, err, status) =
    run_rom(&["build", "--tree", "nixpkgs#hello", "--", "--rebuild"]);
  // Should not error about missing expression
  assert!(
    !err.contains("No package or flake specified for nix build"),
    "should not print missing expression error, got: {}",
    err
  );
  // Status may be 0 or nonzero depending on nix, but should not be our error
}
