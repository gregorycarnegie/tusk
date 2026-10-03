pub mod batch;
pub mod net2;
pub mod people;
pub mod protocol;
pub mod sheet;
pub mod token;
pub mod tsql;

#[cfg(target_arch = "wasm32")]
mod batch_client;
#[cfg(target_arch = "wasm32")]
mod client;
#[cfg(target_arch = "wasm32")]
mod net2_client;
#[cfg(target_arch = "wasm32")]
mod people_client;
#[cfg(target_arch = "wasm32")]
mod portrait_client;
#[cfg(target_arch = "wasm32")]
mod reader;
