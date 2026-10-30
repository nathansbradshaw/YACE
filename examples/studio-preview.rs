//! Run the shared UI against a disposable project on an isolated loopback port.
#[cfg(feature = "browser")]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut args = std::env::args_os().skip(1);
    let project = args.next().context("Pass a disposable project directory")?;
    let port = args
        .next()
        .map(|value| value.to_string_lossy().parse::<u16>())
        .transpose()?
        .unwrap_or(3050);
    chart_engine::browser::serve_project(std::path::Path::new(&project), port)
}

#[cfg(not(feature = "browser"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Run with --no-default-features --features browser")
}
