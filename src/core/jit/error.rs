use derive_more::From;

#[derive(From)]
pub enum Error {
    #[from(ignore)]
    BuildError(String),
    ParseError(async_graphql::parser::Error),
}

pub type Result<A> = std::result::Result<A, Error>;
