use std::collections::HashMap;
use std::ops::DerefMut;

use async_graphql::parser::types::ExecutableDocument;
use async_graphql::Variables;
use async_graphql_value::ConstValue;
use hyper::Body;

use super::path::Path;
use super::Result;
use crate::core::async_graphql_hyper::ParsedGraphQLRequest;

/// A partial GraphQLRequest that contains a parsed executable GraphQL document.
#[derive(Debug)]
pub struct PartialRequest<'a> {
    pub body: Option<&'a String>,
    pub doc: &'a ExecutableDocument,
    pub variables: Variables,
    pub path: &'a Path,
}

impl PartialRequest<'_> {
    pub async fn into_request(mut self, body: Body) -> Result<ParsedGraphQLRequest> {
        let variables = std::mem::take(self.variables.deref_mut());
        let mut variables =
            HashMap::from_iter(variables.into_iter().map(|(k, v)| (k.to_string(), v)));

        if let Some(key) = self.body {
            let bytes = hyper::body::to_bytes(body).await?;
            let body: ConstValue = serde_json::from_slice(&bytes)?;
            variables.insert(key.to_string(), body);
        }

        Ok(ParsedGraphQLRequest {
            // use path as query because query is used as part of the hashing
            // and we need to have different hashed for different operations
            // TODO: is there any way to make it more explicit here?
            query: self.path.as_str().to_string(),
            operation_name: None,
            variables,
            extensions: Default::default(),
            parsed_query: self.doc.clone(),
        })
    }
}
