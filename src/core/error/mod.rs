pub mod cache;
pub mod worker;

use derive_more::From;

#[derive(From, thiserror::Error, Debug)]
pub enum Error {
    #[error("Worker Error")]
    Worker(worker::Error),
}

pub type Result<A, E> = std::result::Result<A, E>;
