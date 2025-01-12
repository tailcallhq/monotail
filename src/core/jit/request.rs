use std::collections::HashMap;

use async_graphql::parser::types::ExecutableDocument;
use async_graphql_value::ConstValue;
use tailcall_valid::Validator;

use super::{transform, Builder, OperationPlan, Result, Variables};
use crate::core::async_graphql_hyper::{GraphQLRequest, ParsedGraphQLRequest};
use crate::core::blueprint::Blueprint;
use crate::core::transform::TransformerOps;
use crate::core::Transform;

#[derive(Debug, Clone)]
pub struct Request<V> {
    pub query: String,
    pub operation_name: Option<String>,
    pub variables: Variables<V>,
    pub extensions: HashMap<String, V>,
    pub parsed_query: ExecutableDocument,
}

impl TryFrom<GraphQLRequest> for Request<ConstValue> {
    type Error = super::Error;

    fn try_from(value: GraphQLRequest) -> Result<Self> {
        let value = ParsedGraphQLRequest::try_from(value)?;

        Self::try_from(value)
    }
}

impl TryFrom<ParsedGraphQLRequest> for Request<ConstValue> {
    type Error = super::Error;
    fn try_from(value: ParsedGraphQLRequest) -> Result<Self> {
        Ok(Self {
            parsed_query: value.parsed_query,
            query: value.query,
            operation_name: value.operation_name,
            variables: Variables::from(value.variables),
            extensions: value.extensions,
        })
    }
}

impl Request<ConstValue> {
    pub fn create_plan(
        &self,
        blueprint: &Blueprint,
    ) -> Result<OperationPlan<async_graphql_value::Value>> {
        let builder = Builder::new(blueprint, &self.parsed_query);
        let plan = builder.build(self.operation_name.as_deref())?;

        transform::CheckConst::new()
            .pipe(transform::CheckProtected::new())
            .pipe(transform::AuthPlanner::new())
            .pipe(transform::CheckDedupe::new())
            .pipe(transform::CheckCache::new())
            .pipe(transform::GraphQL::new())
            .transform(plan)
            .to_result()
            // both transformers are infallible right now
            // but we can't just unwrap this in stable rust
            // so convert to the Unknown error
            .map_err(|_| super::Error::Unknown)
    }
}

impl<V> Request<V> {
    pub fn new(query: &str) -> Self {
        Self {
            query: query.to_string(),
            operation_name: None,
            variables: Variables::new(),
            extensions: HashMap::new(),
            parsed_query: async_graphql::parser::parse_query(query).unwrap(),
        }
    }

    pub fn variables(self, vars: impl IntoIterator<Item = (String, V)>) -> Self {
        Self { variables: Variables::from_iter(vars), ..self }
    }
}
