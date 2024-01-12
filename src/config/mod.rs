pub use config::*;
pub use key_values::*;
pub use resolver::*;
pub use server::*;
pub use source::*;

mod config;
mod from_document;
pub mod group_by;
mod into_document;
mod key_values;
mod n_plus_one;
pub mod reader;
mod resolver;
mod server;
mod source;
mod writer;

fn is_default<T: Default + Eq>(val: &T) -> bool {
  *val == T::default()
}
