use async_graphql::parser::types::OperationType;
use thiserror::Error;

use super::graphql_error::ErrorExtensions;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("Error while building the plan")]
pub enum BuildError {
    #[error("Root Operation type not defined for {operation}")]
    RootOperationTypeNotDefined { operation: OperationType },
    #[error("ResolveInputError: {0}")]
    ResolveInputError(#[from] ResolveInputError),
    #[error(r#"Unknown operation named "{0}""#)]
    OperationNotFound(String),
    #[error("Operation name required in request")]
    OperationNameRequired,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("Cannot resolve the input value")]
pub enum ResolveInputError {
    #[error("Variable `{0}` is not defined")]
    VariableIsNotFound(String),
    #[error("Argument `{arg_name}` for field `{field_name}` is required")]
    ArgumentIsRequired {
        arg_name: String,
        field_name: String,
    },
}

#[derive(Error, Debug, Clone)]
pub enum ValidationError {
    // TODO: replace with sane error message. Right now, it's defined as is only for compatibility
    // with async_graphql error message for this case
    #[error(r#"internal: invalid value for scalar "{type_of}", expected "FieldValue::Value""#)]
    ScalarInvalid { type_of: String },
    #[error(r#"internal: invalid item for enum "{type_of}""#)]
    EnumInvalid { type_of: String },
    #[error("internal: non-null types require a return value")]
    ValueRequired,
}

#[derive(Debug, Clone, Error)]
pub enum Error {
    #[error("Build error: {0}")]
    BuildError(#[from] BuildError),
    #[error("ParseError: {0}")]
    ParseError(#[from] async_graphql::parser::Error),
    #[error(transparent)]
    IR(#[from] crate::core::ir::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("{0}")]
    ServerError(async_graphql::ServerError),
    #[error("Unexpected error")]
    Unknown,
}

impl From<async_graphql::ServerError> for Error {
    fn from(value: async_graphql::ServerError) -> Self {
        Self::ServerError(value)
    }
}

impl ErrorExtensions for Error {
    fn extend(&self) -> super::graphql_error::Error {
        match self {
            Error::BuildError(error) => error.extend(),
            Error::ParseError(error) => error.extend(),
            Error::IR(error) => error.extend(),
            Error::Validation(error) => error.extend(),
            Error::ServerError(error) => error.extend(),
            Error::Unknown => super::graphql_error::Error::new(self.to_string()),
        }
    }
}

pub type Result<A> = std::result::Result<A, Error>;
