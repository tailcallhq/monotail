use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use async_graphql::Value;
use async_graphql_value::{ConstValue, Extensions};
use derive_setters::Setters;
use futures_util::stream::FuturesOrdered;
use futures_util::StreamExt;
use tailcall_hasher::TailcallHasher;

use super::{AnyResponse, BatchResponse, Response};
use crate::core::app_context::AppContext;
use crate::core::async_graphql_hyper::{BatchWrapper, GraphQLRequest, OperationId};
use crate::core::http::RequestContext;
use crate::core::jit::{self, ConstValueExecutor, OPHash};

#[derive(Clone, Setters)]
pub struct JITExecutor {
    app_ctx: Arc<AppContext>,
    req_ctx: Arc<RequestContext>,
    operation_id: OperationId,
    flatten_response: bool,
}

impl JITExecutor {
    pub fn new(
        app_ctx: Arc<AppContext>,
        req_ctx: Arc<RequestContext>,
        operation_id: OperationId,
    ) -> Self {
        Self { app_ctx, req_ctx, operation_id, flatten_response: false }
    }

    #[inline(always)]
    async fn exec(
        &self,
        exec: ConstValueExecutor,
        jit_request: jit::Request<ConstValue>,
    ) -> AnyResponse<Vec<u8>> {
        exec.execute(&self.app_ctx, &self.req_ctx, jit_request)
            .await
    }

    #[inline(always)]
    async fn dedupe_and_exec(
        &self,
        exec: ConstValueExecutor,
        jit_request: jit::Request<ConstValue>,
    ) -> AnyResponse<Vec<u8>> {
        let out = self
            .app_ctx
            .dedupe_operation_handler
            .dedupe(&self.operation_id, || {
                Box::pin(async move {
                    let resp = self.exec(exec, jit_request).await;
                    Ok(resp)
                })
            })
            .await;

        out.unwrap_or_default()
    }

    /// Calculates hash for the request considering
    /// the request is const, i.e. doesn't depend on input.
    /// That's basically use only the query itself to calculating the hash
    #[inline(always)]
    fn const_execution_hash<T>(request: &jit::Request<T>) -> OPHash {
        let hasher = &mut TailcallHasher::default();

        request.query.hash(hasher);

        OPHash::new(hasher.finish())
    }
}

impl JITExecutor {
    pub async fn execute<T>(&self, request: T) -> AnyResponse<Vec<u8>>
    where
        jit::Request<ConstValue>: TryFrom<T, Error = super::Error>,
        T: Hash + Send + 'static,
    {
        let jit_request = match jit::Request::try_from(request) {
            Ok(request) => request,
            Err(error) => return Response::<ConstValue>::from(error).into(),
        };

        let const_execution_hash = Self::const_execution_hash(&jit_request);

        // check if the request is has been set to const_execution_cache
        // and if yes serve the response from the cache since
        // the query doesn't depend on input and could be calculated once
        // WARN: make sure the value is set to cache only if the plan is actually
        // is_const
        if let Some(response) = self
            .app_ctx
            .const_execution_cache
            .get(&const_execution_hash)
        {
            return response.clone();
        }
        let exec = if let Some(op) = self.app_ctx.operation_plans.get(&const_execution_hash) {
            ConstValueExecutor::from(op.value().clone())
        } else {
            let exec = match ConstValueExecutor::try_new(&jit_request, &self.app_ctx) {
                Ok(exec) => exec,
                Err(error) => return Response::<ConstValue>::from(error).into(),
            };
            self.app_ctx
                .operation_plans
                .insert(const_execution_hash.clone(), exec.plan.clone());
            exec
        };

        let exec = exec.flatten_response(self.flatten_response);
        let is_const = exec.plan.is_const;
        let is_protected = exec.plan.is_protected;

        let response = if exec.plan.can_dedupe() {
            self.dedupe_and_exec(exec, jit_request).await
        } else {
            self.exec(exec, jit_request).await
        };

        // Cache the response if it's constant and not wrapped with protected.
        if is_const && !is_protected {
            self.app_ctx
                .const_execution_cache
                .insert(const_execution_hash, response.clone());
        }

        response
    }

    /// Execute a GraphQL batch query.
    pub async fn execute_batch(
        &self,
        batch_request: BatchWrapper<GraphQLRequest>,
    ) -> BatchResponse<Vec<u8>> {
        match batch_request {
            BatchWrapper::Single(request) => BatchResponse::Single(self.execute(request).await),
            BatchWrapper::Batch(requests) => {
                let futs = FuturesOrdered::from_iter(
                    requests.into_iter().map(|request| self.execute(request)),
                );
                let responses = futs.collect::<Vec<_>>().await;
                BatchResponse::Batch(responses)
            }
        }
    }
}

// TODO: used only for introspection, simplify somehow?
impl From<jit::Request<Value>> for async_graphql::Request {
    fn from(value: jit::Request<Value>) -> Self {
        let mut request = async_graphql::Request::new(value.query);
        request.variables.extend(
            value
                .variables
                .into_hashmap()
                .into_iter()
                .map(|(k, v)| (async_graphql::Name::new(k), v))
                .collect::<BTreeMap<_, _>>(),
        );
        request.extensions = Extensions(value.extensions);
        request.operation_name = value.operation_name;
        request
    }
}
