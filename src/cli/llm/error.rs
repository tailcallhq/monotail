use strum_macros::Display;

use derive_more::From;

#[derive(Debug, From, Display, thiserror::Error)]
pub enum Error {
    GenAI(genai::Error),
    EmptyResponse,
    Serde(serde_json::Error),
}

pub type Result<A> = std::result::Result<A, Error>;
