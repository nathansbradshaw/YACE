// Keep generation workers and all CLI actions identical to the desktop engine,
// while giving browser builds their own executable so native packaging is safe.
include!("../main.rs");
