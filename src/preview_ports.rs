//! A shared allowlist keeps preview servers bounded across YACE worktrees.
pub const PRIMARY: u16 = 3030;
pub const SECONDARY: u16 = 3032;

pub fn validate(port: u16) -> Result<(), String> {
    if [PRIMARY, SECONDARY].contains(&port) {
        Ok(())
    } else {
        Err(format!(
            "YACE previews may use only {PRIMARY} (primary) or {SECONDARY} (secondary); port {port} is not allowed. Reuse the existing owner, coordinate a handoff, or wait. Do not choose another port."
        ))
    }
}
