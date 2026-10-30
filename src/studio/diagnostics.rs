//! Retain desktop panic details: Finder does not keep the process's stderr.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(home) = std::env::var_os("HOME") {
            let directory = std::path::PathBuf::from(home).join("Library/Logs/YACE");
            if std::fs::create_dir_all(&directory).is_ok() {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let path = directory.join(format!("panic-{stamp}-{}.log", std::process::id()));
                let _ = std::fs::write(
                    path,
                    format!("{info}\n\n{}\n", std::backtrace::Backtrace::force_capture()),
                );
            }
        }
        previous(info);
    }));
}
