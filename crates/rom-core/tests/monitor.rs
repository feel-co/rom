use rom_core::{
  monitor::{Monitor, extract_byte_size, extract_path_from_message},
  types::Config,
};

#[test]
fn test_monitor_creation() {
  let config = Config::default();
  let output = Vec::new();
  let monitor = Monitor::new(config, output);
  assert!(monitor.is_ok());
}

#[test]
fn test_extract_path_from_message() {
  let line = "building '/nix/store/abc123-hello-1.0.drv'";
  let path = extract_path_from_message(line);
  assert!(path.is_some());
  assert!(path.unwrap().contains("hello-1.0.drv"));
}

#[test]
fn test_extract_path_unquoted() {
  let line = "building /nix/store/abc123-hello-1.0.drv locally";
  let path = extract_path_from_message(line);
  assert!(path.is_some());
}

#[test]
fn test_extract_byte_size() {
  let line = "downloaded 123 KiB in 2 seconds";
  assert_eq!(extract_byte_size(line), Some(123 * 1024));

  let line2 = "downloading 4.5 MiB";
  assert_eq!(
    extract_byte_size(line2),
    Some((4.5 * 1024.0 * 1024.0) as u64)
  );

  let line3 = "no size here";
  assert_eq!(extract_byte_size(line3), None);
}
