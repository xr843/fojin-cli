pub mod cli;
pub mod data;
pub mod model;
pub mod normalize;
pub mod query;
pub mod render;
pub mod schema;

pub use cli::run;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
