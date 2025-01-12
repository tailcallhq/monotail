use std::collections::BTreeMap;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use async_graphql::{BatchRequest, Value};
use async_graphql_value::{ConstValue, Extensions};
use futures_util::stream::FuturesOrdered;
use futures_util::StreamExt;
use tailcall_hasher::TailcallHasher;

use super::{AnyResponse, BatchResponse, Response};
use crate::core::app_context::AppContext;
use crate::core::async_graphql_hyper::OperationId;
use crate::core::helpers::value::arc_result_to_result;
use crate::core::http::RequestContext;
use crate::core::jit::{self, ConstValueExecutor, OPHash, Pos, Positioned};

#[derive(Clone)]
pub struct JITExecutor {
    app_ctx: Arc<AppContext>,
    req_ctx: Arc<RequestContext>,
    operation_id: OperationId,
}

impl JITExecutor {
    pub fn new(
        app_ctx: Arc<AppContext>,
        req_ctx: Arc<RequestContext>,
        operation_id: OperationId,
    ) -> Self {
        Self { app_ctx, req_ctx, operation_id }
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
                    Arc::new(Ok(resp))
                })
            })
            .await;

        arc_result_to_result(out).unwrap_or_default()
    }

    #[inline(always)]
    fn req_hash(request: &async_graphql::Request) -> OPHash {
        let mut hasher = TailcallHasher::default();
        request.query.hash(&mut hasher);

        OPHash::new(hasher.finish())
    }
}

impl JITExecutor {
    pub fn execute(
        &self,
        request: async_graphql::Request,
    ) -> impl Future<Output = AnyResponse<Vec<u8>>> + Send + '_ {
        // TODO: hash considering only the query itself ignoring specified operation and
        // variables that could differ for the same query
        let hash = Self::req_hash(&request);

        async move {
            if let Some(response) = self.app_ctx.const_execution_cache.get(&hash) {
                return response.clone();
            }

            let jit_request = jit::Request::from(request);
            let exec = if let Some(op) = self.app_ctx.operation_plans.get(&hash) {
                ConstValueExecutor::from(op.value().clone())
            } else {
                let exec = match ConstValueExecutor::try_new(&jit_request, &self.app_ctx) {
                    Ok(exec) => exec,
                    Err(error) => {
                        return Response::<async_graphql::Value>::default()
                            .with_errors(vec![Positioned::new(error, Pos::default())])
                            .into()
                    }
                };
                self.app_ctx
                    .operation_plans
                    .insert(hash.clone(), exec.plan.clone());
                exec
            };

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
                    .insert(hash, response.clone());
            }

            response
        }
    }

    /// Execute a GraphQL batch query.
    pub async fn execute_batch(&self, batch_request: BatchRequest) -> BatchResponse<Vec<u8>> {
        match batch_request {
            BatchRequest::Single(request) => BatchResponse::Single(self.execute(request).await),
            BatchRequest::Batch(requests) => {
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
