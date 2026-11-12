mod bind;
mod context;
mod error;
mod init;

pub const PRELUDE: &str = include_str!("./prelude.js");

pub use bind::{FromJsValue, IntoJs, IntoNative, Rest};
pub use context::V8Context;
pub use init::init_v8_context;
