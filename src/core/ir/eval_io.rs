use std::sync::Arc;

use async_graphql_value::ConstValue;

use super::eval_http::{
    execute_grpc_request_with_dl, execute_raw_grpc_request, execute_raw_request,
    execute_request_with_dl, parse_graphql_response, set_headers, EvalHttp, WorkerContext,
};
use super::model::{CacheKey, IO};
use super::{DynamicRequest, EvalContext, ResolverContextLike};
use crate::core::config::GraphQLOperationType;
use crate::core::data_loader::DataLoader;
use crate::core::graphql::GraphqlDataLoader;
use crate::core::grpc;
use crate::core::grpc::data_loader::GrpcDataLoader;
use crate::core::http::DataLoaderRequest;
use crate::core::ir::Error;

pub async fn eval_io<Ctx>(io: &IO, ctx: &mut EvalContext<'_, Ctx>) -> Arc<Result<ConstValue, Error>>
where
    Ctx: ResolverContextLike + Sync,
{
    // Note: Handled the case separately for performance reasons. It avoids cache
    // key generation when it's not required
    let dedupe = io.dedupe();

    if !dedupe || !ctx.is_query() {
        return eval_io_inner_arc(io, ctx).await;
    }
    if let Some(key) = io.cache_key(ctx) {
        ctx.request_ctx
            .cache
            .dedupe(&key, || async {
                ctx.request_ctx
                    .dedupe_handler
                    .dedupe(&key, || eval_io_inner_arc(io, ctx))
                    .await
            })
            .await
    } else {
        eval_io_inner_arc(io, ctx).await
    }
}

async fn eval_io_inner<Ctx>(io: &IO, ctx: &mut EvalContext<'_, Ctx>) -> Result<ConstValue, Error>
where
    Ctx: ResolverContextLike + Sync,
{
    match io {
        IO::Http { req_template, dl_id, hook, .. } => {
            let event_worker = &ctx.request_ctx.runtime.cmd_worker;
            let js_worker = &ctx.request_ctx.runtime.worker;
            let eval_http = EvalHttp::new(ctx, req_template, dl_id);
            let request = eval_http.init_request()?;
            let response = match (&event_worker, js_worker, hook) {
                (Some(worker), Some(js_worker), Some(hook)) => {
                    let worker_ctx = WorkerContext::new(worker, js_worker, hook);
                    eval_http.execute_with_worker(request, worker_ctx).await?
                }
                _ => eval_http.execute(request).await?,
            };

            Ok(response.body)
        }
        IO::GraphQL { req_template, field_name, dl_id, .. } => {
            let req = req_template.to_request(ctx)?;
            let request = DynamicRequest::new(req);
            let res = if ctx.request_ctx.upstream.batch.is_some()
                && matches!(req_template.operation_type, GraphQLOperationType::Query)
            {
                let data_loader: Option<&DataLoader<DataLoaderRequest, GraphqlDataLoader>> =
                    dl_id.and_then(|dl| ctx.request_ctx.gql_data_loaders.get(dl.as_usize()));
                execute_request_with_dl(ctx, request, data_loader).await?
            } else {
                execute_raw_request(ctx, request).await?
            };

            set_headers(ctx, &res);
            parse_graphql_response(ctx, res, field_name)
        }
        IO::Grpc { req_template, dl_id, hook, .. } => {
            let rendered = req_template.render(ctx)?;
            let worker = &ctx.request_ctx.runtime.worker;

            let res = if ctx.request_ctx.upstream.batch.is_some() &&
                    // TODO: share check for operation_type for resolvers
                    matches!(req_template.operation_type, GraphQLOperationType::Query)
            {
                let data_loader: Option<&DataLoader<grpc::DataLoaderRequest, GrpcDataLoader>> =
                    dl_id.and_then(|index| ctx.request_ctx.grpc_data_loaders.get(index.as_usize()));
                execute_grpc_request_with_dl(ctx, rendered, data_loader).await?
            } else {
                let req = rendered.to_request()?;
                execute_raw_grpc_request(ctx, req, &req_template.operation).await?
            };

            let res = match (worker.as_ref(), hook.as_ref()) {
                (Some(worker), Some(hook)) => hook.on_response(worker, res).await?,
                _ => res,
            };
            set_headers(ctx, &res);

            Ok(res.body)
        }
        IO::Js { name } => {
            if let Some((worker, value)) = ctx
                .request_ctx
                .runtime
                .worker
                .as_ref()
                .zip(ctx.value().cloned())
            {
                let val = worker.call(name, value).await?;
                Ok(val.unwrap_or_default())
            } else {
                Ok(ConstValue::Null)
            }
        }
    }
}

async fn eval_io_inner_arc<Ctx>(
    io: &IO,
    ctx: &mut EvalContext<'_, Ctx>,
) -> Arc<Result<ConstValue, Error>>
where
    Ctx: ResolverContextLike + Sync,
{
    return Arc::new(eval_io_inner(io, ctx).await);
}
