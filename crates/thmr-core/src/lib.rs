pub mod audio;
pub mod client;
pub mod protocol;
pub mod runtime;
pub mod settings;

pub const VERSION: &str = env!("THMR_VERSION");
pub const BUILD_NUMBER: &str = env!("THMR_BUILD");
pub const PROTOCOL_VERSION: u32 = 1;
