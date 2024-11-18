mod blueprint;
mod compress;
mod cors;
mod definitions;
mod directive;
mod dynamic_value;
mod from_config;
mod index;
mod interface_resolver;
mod into_document;
mod into_schema;
mod links;
mod mustache;
mod operators;
mod schema;
mod server;
mod timeout;
mod union_resolver;
mod upstream;

pub use blueprint::*;
pub use cors::*;
pub use definitions::*;
pub use dynamic_value::*;
pub use from_config::*;
pub use index::*;
pub use links::*;
pub use operators::*;
pub use schema::*;
pub use server::*;
pub use timeout::GlobalTimeout;
pub use upstream::*;

use crate::core::config::ConfigModule;
use crate::core::try_fold::TryFold;

pub type TryFoldConfig<'a, A> = TryFold<'a, ConfigModule, A, String>;
