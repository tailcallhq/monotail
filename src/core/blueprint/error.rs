use std::net::AddrParseError;

use async_graphql::dynamic::SchemaError;
use tailcall_valid::{Cause, ValidationError};

use crate::core::Errata;

#[derive(Debug, thiserror::Error)]
pub enum BlueprintError {
    #[error("Apollo federation resolvers can't be a part of entity resolver")]
    ApolloFederationResolversNoPartOfEntityResolver,

    #[error("Query type is not an object inside the blueprint")]
    QueryTypeNotObject,

    #[error("Cannot find type {0} in the config")]
    TypeNotFoundInConfig(String),

    #[error("Cannot find field {0} in the type")]
    FieldNotFoundInType(String),

    #[error("no argument '{0}' found")]
    ArgumentNotFound(String),

    #[error("field {0} has no resolver")]
    FieldHasNoResolver(String),

    #[error("Steps can't be empty")]
    StepsCanNotBeEmpty,

    #[error("Result resolver can't be empty")]
    ResultResolverCanNotBeEmpty,

    #[error("call must have query or mutation")]
    CallMustHaveQueryOrMutation,

    #[error("invalid JSON: {0}")]
    InvalidJson(anyhow::Error),

    #[error("field {0} not found")]
    FieldNotFound(String),

    #[error("Invalid method format: {0}. Expected format is <package>.<service>.<method>")]
    InvalidGrpcMethodFormat(String),

    #[error("Protobuf files were not specified in the config")]
    ProtobufFilesNotSpecifiedInConfig,

    #[error("GroupBy is only supported for GET and POST requests")]
    GroupByOnlyForGetAndPost,

    #[error("Request body batching requires exactly one dynamic value in the body.")]
    BatchRequiresDynamicParameter,

    #[error("Batching capability was used without enabling it in upstream")]
    IncorrectBatchingUsage,

    #[error("batchKey requires either body or query parameters")]
    BatchKeyRequiresEitherBodyOrQuery,

    #[error("script is required")]
    ScriptIsRequired,

    #[error("Field is already implemented from interface")]
    FieldExistsInInterface,

    #[error("Input types can not be protected")]
    InputTypesCannotBeProtected,

    #[error("@protected operator is used but there is no @link definitions for auth providers")]
    ProtectedOperatorNoAuthProviders,

    #[error("Auth provider {0} not found")]
    AuthProviderNotFound(String),

    #[error("syntax error when parsing `{0}`")]
    SyntaxErrorWhenParsing(String),

    #[error("Scalar type {0} is predefined")]
    ScalarTypeIsPredefined(String),

    #[error("Undeclared type '{0}' was found")]
    UndeclaredTypeFound(String),

    #[error("Cannot add field")]
    CannotAddField,

    #[error("Path [{0}] does not exist")]
    PathDoesNotExist(String),

    #[error("Path: [{0}] contains resolver {1} at [{2}.{3}]")]
    PathContainsResolver(String, String, String, String),

    #[error("Could not find field {0} in path {1}")]
    FieldNotFoundInPath(String, String),

    #[error("No variants found for enum")]
    NoVariantsFoundForEnum,

    #[error("Link src cannot be empty")]
    LinkSrcCannotBeEmpty,

    #[error("Duplicated id: {0}")]
    Duplicated(String),

    #[error("Only one script link is allowed")]
    OnlyOneScriptLinkAllowed,

    #[error("Only one key link is allowed")]
    OnlyOneKeyLinkAllowed,

    #[error("no value '{0}' found")]
    NoValueFound(String),

    #[error("value '{0}' is a nullable type")]
    ValueIsNullableType(String),

    #[error("value '{0}' is not of a scalar type")]
    ValueIsNotOfScalarType(String),

    #[error("no type '{0}' found")]
    NoTypeFound(String),

    #[error("too few parts in template")]
    TooFewPartsInTemplate,

    #[error("can't use list type '{0}' here")]
    CantUseListTypeHere(String),

    #[error("argument '{0}' is a nullable type")]
    ArgumentIsNullableType(String),

    #[error("var '{0}' is not set in the server config")]
    VarNotSetInServerConfig(String),

    #[error("unknown template directive '{0}'")]
    UnknownTemplateDirective(String),

    #[error("Query type is not defined")]
    QueryTypeNotDefined,

    #[error("No resolver has been found in the schema")]
    NoResolverFoundInSchema,

    #[error("Mutation type is not defined")]
    MutationTypeNotDefined,

    #[error("Certificate is required for HTTP2")]
    CertificateIsRequiredForHTTP2,

    #[error("Key is required for HTTP2")]
    KeyIsRequiredForHTTP2,

    #[error("Experimental headers must start with 'x-' or 'X-'. Got: '{0}'")]
    ExperimentalHeaderInvalidFormat(String),

    #[error("`graph_ref` should be in the format <graph_id>@<variant> where `graph_id` and `variant` can only contain letters, numbers, '-' and '_'. Found {0}")]
    InvalidGraphRef(String),

    #[error("Invalid CORS configuration: Cannot combine `Access-Control-Allow-Credentials: true` with `{0}: *`")]
    InvalidCORSConfiguration(String),

    #[error("{0}")]
    Cause(String),

    #[error("{0}")]
    Description(String),

    #[error("Parsing failed because of {0}")]
    ParsingFailed(#[from] AddrParseError),

    #[error(transparent)]
    Schema(#[from] SchemaError),

    #[error(transparent)]
    UrlParse(#[from] url::ParseError),

    #[error("Parsing failed because of {0}")]
    InvalidHeaderName(#[from] http::header::InvalidHeaderName),

    #[error("Parsing failed because of {0}")]
    InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),

    #[error(transparent)]
    Error(#[from] anyhow::Error),
}

impl PartialEq for BlueprintError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl From<ValidationError<crate::core::blueprint::BlueprintError>> for Errata {
    fn from(error: ValidationError<crate::core::blueprint::BlueprintError>) -> Self {
        Errata::new("Blueprint Error").caused_by(
            error
                .as_vec()
                .iter()
                .map(|cause| {
                    let mut err =
                        Errata::new(&cause.message.to_string()).trace(cause.trace.clone().into());
                    if let Some(description) = &cause.description {
                        err = err.description(description.to_string());
                    }
                    err
                })
                .collect(),
        )
    }
}

impl BlueprintError {
    pub fn to_validation_string(
        errors: ValidationError<BlueprintError>,
    ) -> ValidationError<String> {
        let causes: Vec<Cause<_>> = errors
            .as_vec()
            .iter()
            .map(|cause| {
                let new_cause =
                    Cause::new(cause.message.to_string()).trace(cause.trace.clone().into());

                if let Some(description) = &cause.description {
                    new_cause.description(description.to_string())
                } else {
                    new_cause
                }
            })
            .collect();

        ValidationError::from(causes)
    }

    pub fn from_validation_str(errors: ValidationError<&str>) -> ValidationError<BlueprintError> {
        let causes: Vec<Cause<_>> = errors
            .as_vec()
            .iter()
            .map(|cause| {
                let new_cause = Cause::new(BlueprintError::Cause(cause.message.to_string()))
                    .trace(cause.trace.clone().into());

                if let Some(description) = cause.description {
                    new_cause.description(BlueprintError::Description(description.to_string()))
                } else {
                    new_cause
                }
            })
            .collect();

        ValidationError::from(causes)
    }

    pub fn from_validation_string(
        errors: ValidationError<String>,
    ) -> ValidationError<BlueprintError> {
        let causes: Vec<Cause<_>> = errors
            .as_vec()
            .iter()
            .map(|cause| {
                let new_cause = Cause::new(BlueprintError::Cause(cause.message.to_string()))
                    .trace(cause.trace.clone().into());

                if let Some(description) = &cause.description {
                    new_cause.description(BlueprintError::Description(description.to_string()))
                } else {
                    new_cause
                }
            })
            .collect();

        ValidationError::from(causes)
    }
}
