fn main() -> eyre::Result<()> {
  // Initialize tracing
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rom=warn")),
    )
    .with_target(false)
    .with_writer(std::io::stderr)
    .init();

  // Run the CLI
  rom::run()
}
