use std::sync::Arc;

use async_graphql_value::{ConstValue, Value};
use derive_setters::Setters;
use futures_util::future::join_all;
use tailcall_valid::Validator;

use super::context::Context;
use super::exec::{Executor, IRExecutor};
use super::graphql_error::GraphQLError;
use super::{transform, AnyResponse, BuildError, Error, OperationPlan, Request, Response, Result};
use crate::core::app_context::AppContext;
use crate::core::http::RequestContext;
use crate::core::ir::model::IR;
use crate::core::ir::{self, EmptyResolverContext, EvalContext};
use crate::core::jit::synth::Synth;
use crate::core::jit::transform::InputResolver;
use crate::core::json::{JsonLike, JsonLikeList, JsonObjectLike};
use crate::core::Transform;

/// A specialized executor that executes with async_graphql::Value
#[derive(Setters)]
pub struct ConstValueExecutor {
    pub plan: OperationPlan<Value>,

    flatten_response: bool,
}

impl From<OperationPlan<Value>> for ConstValueExecutor {
    fn from(plan: OperationPlan<Value>) -> Self {
        Self { plan, flatten_response: false }
    }
}

impl ConstValueExecutor {
    pub fn try_new(request: &Request<ConstValue>, app_ctx: &Arc<AppContext>) -> Result<Self> {
        let plan = request.create_plan(&app_ctx.blueprint)?;
        Ok(Self::from(plan))
    }

    pub async fn execute<'a>(
        self,
        app_ctx: &Arc<AppContext>,
        req_ctx: &RequestContext,
        request: Request<ConstValue>,
    ) -> AnyResponse<Vec<u8>> {
        // Run all the IRs in the before chain
        if let Some(ir) = &self.plan.before {
            let mut eval_context = EvalContext::new(req_ctx, &EmptyResolverContext {});
            match ir.eval(&mut eval_context).await {
                Ok(_) => (),
                Err(err) => {
                    let resp: Response<ConstValue> = Response::default();
                    return resp
                        .with_errors(vec![GraphQLError::new(err.to_string(), None)])
                        .into();
                }
            }
        }

        let is_introspection_query =
            req_ctx.server.get_enable_introspection() && self.plan.is_introspection_query;
        let flatten_response = self.flatten_response;
        let variables = &request.variables;

        // Attempt to skip unnecessary fields
        let Ok(plan) = transform::Skip::new(variables)
            .transform(self.plan)
            .to_result()
        else {
            let resp: Response<ConstValue> = Response::default();
            // this shouldn't actually ever happen
            return resp
                .with_errors(vec![GraphQLError::new(Error::Unknown.to_string(), None)])
                .into();
        };

        // Attempt to replace variables in the plan with the actual values
        // TODO: operation from [ExecutableDocument] could contain definitions for
        // default values of arguments. That info should be passed to
        // [InputResolver] to resolve defaults properly
        let result = InputResolver::new(plan).resolve_input(variables);

        let plan = match result {
            Ok(plan) => plan,
            Err(err) => {
                let resp: Response<ConstValue> = Response::default();
                return resp
                    .with_errors(vec![GraphQLError::new(
                        BuildError::from(err).to_string(),
                        None,
                    )])
                    .into();
            }
        };

        let exec = ConstValueExec::new(&plan, req_ctx);
        // PERF: remove this particular clone?
        let vars = request.variables.clone();
        let exe = Executor::new(&plan, exec);
        let store = exe.store().await;
        let synth = Synth::new(&plan, store, vars);

        let resp: Response<serde_json_borrow::Value> = exe.execute(&synth).await;

        if is_introspection_query {
            let async_req = async_graphql::Request::from(request).only_introspection();
            let async_resp = app_ctx.execute(async_req).await;

            to_any_response(resp.merge_with(&async_resp), flatten_response)
        } else {
            to_any_response(resp, flatten_response)
        }
    }
}

fn to_any_response(
    resp: Response<serde_json_borrow::Value>,
    flatten: bool,
) -> AnyResponse<Vec<u8>> {
    if flatten {
        if resp.errors.is_empty() {
            AnyResponse {
                body: Arc::new(
                    serde_json::to_vec(flatten_response(&resp.data)).unwrap_or_default(),
                ),
                is_ok: true,
                cache_control: resp.cache_control,
            }
        } else {
            AnyResponse {
                body: Arc::new(serde_json::to_vec(&resp).unwrap_or_default()),
                is_ok: false,
                cache_control: resp.cache_control,
            }
        }
    } else {
        resp.into()
    }
}

fn flatten_response<'a, T: JsonLike<'a>>(data: &'a T) -> &'a T {
    match data.as_object() {
        Some(obj) if obj.len() == 1 => flatten_response(obj.iter().next().unwrap().1),
        _ => data,
    }
}

struct ConstValueExec<'a> {
    plan: &'a OperationPlan<ConstValue>,
    req_context: &'a RequestContext,
}

impl<'a> ConstValueExec<'a> {
    pub fn new(plan: &'a OperationPlan<ConstValue>, req_context: &'a RequestContext) -> Self {
        Self { req_context, plan }
    }

    async fn call(
        &self,
        ctx: &'a Context<'a, <Self as IRExecutor>::Input, <Self as IRExecutor>::Output>,
        ir: &'a IR,
    ) -> Result<<Self as IRExecutor>::Output>
    where
        <Self as IRExecutor>::Input: JsonLike<'a>,
        <Self as IRExecutor>::Output: JsonLike<'a>,
    {
        // if parent value is null do not try to resolve child fields
        if matches!(ctx.value(), Some(v) if v.is_null()) {
            return Ok(Default::default());
        }

        let req_context = &self.req_context;
        let mut eval_ctx = EvalContext::new(req_context, ctx);

        Ok(ir.eval(&mut eval_ctx).await?)
    }
}

impl IRExecutor for ConstValueExec<'_> {
    type Input = ConstValue;
    type Output = ConstValue;
    type Error = Error;

    async fn execute<'a>(
        &'a self,
        ir: &'a IR,
        ctx: &'a Context<'a, Self::Input, Self::Output>,
    ) -> Result<Self::Output> {
        let field = ctx.field();

        match ctx.value() {
            // TODO: check that field is expected list and it's a list of the required deepness
            Some(value) if value.as_array().is_some() => {
                let mut tasks = Vec::new();

                // collect the async tasks first before creating the final result
                value.for_each(&mut |value| {
                    // execute the resolver only for fields that are related to current value
                    // for fragments on union/interface
                    if self.plan.field_is_part_of_value(field, value) {
                        let ctx = ctx.with_value(value);
                        tasks.push(async move { self.call(&ctx, ir).await })
                    }
                });

                let results = join_all(tasks).await;

                let mut iter = results.into_iter();

                // map input value to the calculated results preserving the shape
                // of the input
                Ok(value.map_ref(&mut |value| {
                    // for fragments on union/interface we will
                    // have less entries for resolved values based on the type
                    // pull from the result only field is related and fill with null otherwise
                    if self.plan.field_is_part_of_value(field, value) {
                        iter.next().unwrap_or(Err(ir::Error::IO(
                            "Expected value to be present".to_string(),
                        )
                        .into()))
                    } else {
                        Ok(Self::Output::default())
                    }
                })?)
            }
            _ => Ok(self.call(ctx, ir).await?),
        }
    }
}
