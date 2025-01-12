use std::collections::BTreeMap;
use std::fmt::{Debug, Display, Formatter};

use serde::{Deserialize, Serialize};

use super::{PathSegment, Pos, Positioned};

/// An error in a GraphQL server.
#[derive(Clone, Serialize, Deserialize)]
pub struct GraphQLError {
    /// An explanatory message of the error.
    pub message: String,
    /// Where the error occurred.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub locations: Vec<Pos>,
    /// If the error occurred in a resolver, the path to the error.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub path: Vec<PathSegment<'static>>,
    /// Extensions to the error.
    #[serde(skip_serializing_if = "error_extensions_is_empty", default)]
    pub extensions: Option<ErrorExtensionValues>,
}

impl From<async_graphql::ServerError> for GraphQLError {
    fn from(value: async_graphql::ServerError) -> Self {
        // TODO: remove this once either extension are avail public or we migrate from
        // async_graphql. we can't copy extensions, bcoz it's private inside the
        // async_graphql. hack: serialize the value and deserialize it back to
        // btreemap.
        let extensions = value.extensions.and_then(|ext| {
            serde_json::to_value(ext)
                .ok()
                .and_then(|serialized_value| serde_json::from_value(serialized_value).ok())
                .map(ErrorExtensionValues)
        });

        Self {
            message: value.message,
            locations: value.locations.into_iter().map(|l| l.into()).collect(),
            path: value.path.into_iter().map(|p| p.into()).collect(),
            extensions,
        }
    }
}

impl From<Positioned<super::Error>> for GraphQLError {
    fn from(value: Positioned<super::Error>) -> Self {
        let inner_value = value.value;
        let position = value.pos;

        // async_graphql::parser::Error has special conversion to ServerError
        if let super::Error::ParseError(e) = inner_value {
            return e.into();
        }

        if let super::Error::ServerError(e) = inner_value {
            return e.into();
        }

        let ext = inner_value.extend().extensions;
        let mut server_error = GraphQLError::new(inner_value.to_string(), Some(position));
        server_error.extensions = ext;
        server_error.path = value.path;

        server_error
    }
}

impl GraphQLError {
    /// Create a new server error with the message.
    pub fn new(message: impl Into<String>, pos: Option<Pos>) -> Self {
        Self {
            message: message.into(),
            locations: pos.map(|pos| vec![pos]).unwrap_or_default(),
            path: Vec::new(),
            extensions: None,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn with_path(self, path: Vec<PathSegment<'static>>) -> Self {
        Self { path, ..self }
    }
}

impl Display for GraphQLError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<GraphQLError> for Vec<GraphQLError> {
    fn from(single: GraphQLError) -> Self {
        vec![single]
    }
}

impl From<async_graphql::parser::Error> for GraphQLError {
    fn from(e: async_graphql::parser::Error) -> Self {
        Self {
            message: e.to_string(),
            locations: e.positions().map(|p| p.into()).collect(),
            path: Vec::new(),
            extensions: None,
        }
    }
}

impl Debug for GraphQLError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerError")
            .field("message", &self.message)
            .field("locations", &self.locations)
            .field("path", &self.path)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl PartialEq for GraphQLError {
    fn eq(&self, other: &Self) -> bool {
        self.message.eq(&other.message)
            && self.locations.eq(&other.locations)
            && self.path.eq(&other.path)
            && self.extensions.eq(&other.extensions)
    }
}

fn error_extensions_is_empty(values: &Option<ErrorExtensionValues>) -> bool {
    values.as_ref().map_or(true, |values| values.0.is_empty())
}

/// Extensions to the error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ErrorExtensionValues(BTreeMap<String, async_graphql::Value>);

impl ErrorExtensionValues {
    /// Set an extension value.
    pub fn set(&mut self, name: impl AsRef<str>, value: impl Into<async_graphql::Value>) {
        self.0.insert(name.as_ref().to_string(), value.into());
    }

    /// Unset an extension value.
    pub fn unset(&mut self, name: impl AsRef<str>) {
        self.0.remove(name.as_ref());
    }

    /// Get an extension value.
    pub fn get(&self, name: impl AsRef<str>) -> Option<&async_graphql::Value> {
        self.0.get(name.as_ref())
    }
}

#[derive(Clone, Serialize)]
pub struct Error {
    /// The error message.
    pub message: String,
    /// Extensions to the error.
    #[serde(skip_serializing_if = "error_extensions_is_empty")]
    pub extensions: Option<ErrorExtensionValues>,
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Error")
            .field("message", &self.message)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Delegate to the Debug implementation
        write!(f, "{:?}", self)
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.message.eq(&other.message) && self.extensions.eq(&other.extensions)
    }
}

impl Error {
    /// Create an error from the given error message.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), extensions: None }
    }

    /// Convert the error to a server error.
    #[must_use]
    pub fn into_server_error(self, pos: Pos) -> GraphQLError {
        GraphQLError {
            message: self.message,
            locations: vec![pos],
            path: Vec::new(),
            extensions: self.extensions,
        }
    }
}

// An error which can be extended into a `Error`.
pub trait ErrorExtensions: Sized {
    /// Convert the error to a `Error`.
    fn extend(&self) -> Error;

    /// Add extensions to the error, using a callback to make the extensions.
    fn extend_with<C>(self, cb: C) -> Error
    where
        C: FnOnce(&Self, &mut ErrorExtensionValues),
    {
        let mut new_extensions = Default::default();
        cb(&self, &mut new_extensions);

        let Error { message, extensions } = self.extend();

        let mut extensions = extensions.unwrap_or_default();
        extensions.0.extend(new_extensions.0);

        Error { message, extensions: Some(extensions) }
    }
}

impl ErrorExtensions for Error {
    fn extend(&self) -> Error {
        self.clone()
    }
}

// implementing for &E instead of E gives the user the possibility to implement
// for E which does not conflict with this implementation acting as a fallback.
impl<E: Display> ErrorExtensions for &E {
    fn extend(&self) -> Error {
        Error { message: self.to_string(), extensions: None }
    }
}

#[cfg(test)]
mod test {
    use async_graphql::{ErrorExtensionValues, ServerError};

    #[test]
    fn test_extension_conversion() {
        let mut async_ext = ErrorExtensionValues::default();
        async_ext.set("k-1", async_graphql::Value::Number(2.into()));
        async_ext.set("k-2", async_graphql::Value::Null);
        async_ext.set("k-3", async_graphql::Value::String("test".into()));

        let mut async_server_err = ServerError::new("testing-error-message", None);
        async_server_err.extensions = Some(async_ext);
        let async_ext_str = serde_json::to_value(async_server_err.clone()).unwrap();

        let owned_server_err = super::GraphQLError::from(async_server_err);
        let owned_ext_str = serde_json::to_value(owned_server_err).unwrap();

        assert_eq!(async_ext_str, owned_ext_str);
    }
}
