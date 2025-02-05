use std::sync::Arc;

use async_graphql::dynamic::Schema;
use tailcall_valid::{Cause, Valid, Validator};

use super::{Error, Result};
use crate::core::blueprint::{Blueprint, SchemaModifiers};
use crate::core::http::RequestContext;

#[derive(Debug)]
pub struct OperationQuery {
    query: async_graphql::Request,
}

impl OperationQuery {
    pub fn new(
        query: async_graphql::Request,
        request_context: Arc<RequestContext>,
    ) -> Result<Self> {
        let query = query.data(request_context);
        Ok(Self { query })
    }

    async fn validate(self, schema: &Schema) -> Vec<Error> {
        schema
            .execute(self.query)
            .await
            .errors
            .iter()
            .map(|v| Error::GraphQLServer(v.clone()))
            .collect()
    }
}

pub async fn validate_operations(
    blueprint: &Blueprint,
    operations: Vec<OperationQuery>,
) -> Valid<(), String> {
    let schema = blueprint.to_schema_with(SchemaModifiers::default().with_no_resolver());
    Valid::from_iter(
        futures_util::future::join_all(operations.into_iter().map(|op| op.validate(&schema)))
            .await
            .iter(),
        |errors| {
            if errors.is_empty() {
                Valid::succeed(())
            } else {
                Valid::<(), String>::from_vec_cause(
                    errors.iter().map(|e| Cause::new(e.to_string())).collect(),
                )
            }
        },
    )
    .unit()
}
