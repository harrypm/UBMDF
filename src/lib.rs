//! UBMDF library: the service-port protocol, firmware index, operations and
//! GUI. Used by the desktop binary (src/main.rs) and the Android app
//! (android/, which wraps this crate in a NativeActivity cdylib).

#[cfg(target_os = "android")]
pub mod android;
#[cfg(feature = "bundled-firmware")]
pub mod bundled;
pub mod downloader;
pub mod extract;
#[cfg(feature = "gui")]
pub mod gui;
pub mod index;
pub mod logger;
pub mod manifest;
pub mod ops;
pub mod probe;
pub mod protocol;
pub mod replay;
