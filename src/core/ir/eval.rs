use std::collections::HashMap;
use std::future::Future;
use std::ops::Deref;

use async_graphql_value::ConstValue;
use futures_util::future::join_all;
use indexmap::IndexMap;

use super::eval_io::eval_io;
use super::model::{Cache, CacheKey, Map, IR};
use super::{Error, EvalContext, ResolverContextLike, TypedValue};
use crate::core::auth::verify::{AuthVerifier, Verify};
use crate::core::helpers::value::arc_result_to_result;
use crate::core::json::{JsonLike, JsonObjectLike};
use crate::core::merge_right::MergeRight;
use crate::core::serde_value_ext::ValueExt;

impl IR {
    #[tracing::instrument(skip_all, fields(otel.name = %self), err)]
    pub fn eval<'a, 'b, Ctx>(
        &'a self,
        ctx: &'b mut EvalContext<'a, Ctx>,
    ) -> impl Future<Output = Result<ConstValue, Error>> + Send + use<'a, 'b, Ctx>
    where
        Ctx: ResolverContextLike + Sync,
    {
        Box::pin(async move {
            match self {
                IR::ContextPath(path) => Ok(ctx
                    .path_value(path)
                    .map(|a| a.into_owned())
                    .unwrap_or(async_graphql::Value::Null)),
                IR::Path(input, path) => {
                    let inp = input.eval(ctx).await?;
                    Ok(inp
                        .get_path(path)
                        .unwrap_or(&async_graphql::Value::Null)
                        .clone())
                }
                IR::Dynamic(value) => Ok(value.render_value(ctx)),
                IR::Protect(auth, expr) => {
                    let verifier = AuthVerifier::from(auth.clone());
                    verifier.verify(ctx.request_ctx).await.to_result()?;

                    expr.eval(ctx).await
                }
                IR::IO(io) => arc_result_to_result(eval_io(io, ctx).await),
                IR::Cache(Cache { max_age, io }) => {
                    let io = io.deref();
                    let key = io.cache_key(ctx);
                    if let Some(key) = key {
                        if let Some(val) = ctx.request_ctx.runtime.cache.get(&key).await? {
                            Ok(val)
                        } else {
                            let result = arc_result_to_result(eval_io(io, ctx).await);
                            let val = result?;
                            ctx.request_ctx
                                .runtime
                                .cache
                                .set(key, val.clone(), max_age.to_owned())
                                .await?;
                            Ok(val)
                        }
                    } else {
                        arc_result_to_result(eval_io(io, ctx).await)
                    }
                }
                IR::Map(Map { input, map }) => {
                    fn recursive_map_enum(
                        val: Result<ConstValue, Error>,
                        map: &HashMap<String, String>,
                    ) -> Result<ConstValue, Error> {
                        match val? {
                            ConstValue::Null => Ok(ConstValue::Null),
                            ConstValue::String(key) => {
                                if let Some(value) = map.get(&key) {
                                    Ok(ConstValue::String(value.to_owned()))
                                } else {
                                    Err(Error::ExprEval(format!("Can't find mapped key: {}.", key)))
                                }
                            }
                            ConstValue::List(vec) => {
                                let vec = vec
                                    .into_iter()
                                    .map(|value| recursive_map_enum(Ok(value), map))
                                    .collect::<Result<Vec<_>, _>>()?;
                                Ok(ConstValue::List(vec))
                            }
                            _ => Err(Error::ExprEval(
                                "Mapped key must be either string or array value.".to_owned(),
                            )),
                        }
                    }
                    recursive_map_enum(input.eval(ctx).await, map)
                }
                IR::Pipe(first, second) => {
                    let args = first.eval(&mut ctx.clone()).await?;
                    let ctx = &mut ctx.with_args(args);
                    second.eval(ctx).await
                }
                IR::Merge(vec) => {
                    let results: Vec<_> = join_all(vec.iter().map(|ir| {
                        let mut ctx = ctx.clone();

                        async move { ir.eval(&mut ctx).await }
                    }))
                    .await
                    .into_iter()
                    .collect::<Result<_, _>>()?;

                    // TODO: This is a very opinionated merge. We should allow users to customize
                    // how they would like to merge the values. In future we should support more
                    // merging capabilities by adding an additional parameter to `Merge`.
                    Ok(results
                        .into_iter()
                        .reduce(|acc, result| acc.merge_right(result))
                        .unwrap_or_default())
                }
                IR::Discriminate(discriminator, expr) => expr
                    .eval(ctx)
                    .await
                    .and_then(|value| Ok(discriminator.resolve_type(value)?)),
                IR::Entity(map) => {
                    let representations = ctx.path_arg(&["representations"]);

                    let representations = representations
                        .as_ref()
                        .and_then(|repr| repr.as_array())
                        .ok_or(Error::Entity(
                            "expected `representations` arg as an array of _Any".to_string(),
                        ))?;

                    let mut tasks = Vec::with_capacity(representations.len());
                    for repr in representations {
                        // TODO: combine errors, instead of fail fast?
                        let type_name = repr.get_type_name().ok_or(Error::Entity(
                            "expected __typename to be the part of the representation".to_string(),
                        ))?;

                        let ir = map.get(type_name).ok_or(Error::Entity(format!(
                            "Cannot find a resolver for type: `{type_name}`"
                        )))?;

                        // pass the input for current representation as value in context
                        // TODO: can we drop clone?
                        let mut ctx = ctx.with_value(repr.clone());

                        tasks.push(async move {
                            ir.eval(&mut ctx).await.and_then(|mut value| {
                                // set typename explicitly to reuse it if needed
                                value.set_type_name(type_name.to_owned())?;
                                Ok(value)
                            })
                        });
                    }

                    let result = join_all(tasks).await;

                    let entities = result.into_iter().collect::<Result<_, _>>()?;

                    Ok(ConstValue::List(entities))
                }
                IR::Service(sdl) => {
                    let mut obj = IndexMap::new();

                    obj.insert_key("sdl", ConstValue::string(sdl.into()));

                    Ok(ConstValue::object(obj))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod merge {
        use serde_json::json;

        use super::*;
        use crate::core::blueprint::{Blueprint, DynamicValue};
        use crate::core::http::RequestContext;
        use crate::core::ir::EmptyResolverContext;

        #[tokio::test]
        async fn test_const_values() {
            let a = DynamicValue::Value(
                ConstValue::from_json(json!({
                    "a": 1,
                    "c": {
                        "ca": false
                    }
                }))
                .unwrap(),
            );

            let b = DynamicValue::Value(
                ConstValue::from_json(json!({
                    "b": 2,
                    "c": {
                        "cb": 23
                    }
                }))
                .unwrap(),
            );

            let c = DynamicValue::Value(
                ConstValue::from_json(json!({
                    "c" : {
                        "ca": true,
                        "cc": [1, 2]
                    },
                    "d": "additional"
                }))
                .unwrap(),
            );

            let ir = IR::Merge([a, b, c].into_iter().map(IR::Dynamic).collect());
            let runtime = crate::cli::runtime::init(&Blueprint::default());
            let req_ctx = RequestContext::new(runtime);
            let res_ctx = EmptyResolverContext {};
            let mut eval_ctx = EvalContext::new(&req_ctx, &res_ctx);

            let actual = ir.eval(&mut eval_ctx).await.unwrap();
            let expected = ConstValue::from_json(json!({
                "a": 1,
                "b": 2,
                "c": {
                    "ca": true,
                    "cb": 23,
                    "cc": [1, 2]
                },
                "d": "additional"
            }))
            .unwrap();

            assert_eq!(actual, expected);
        }
    }
}
