use core::future::Future;
use std::pin::Pin;

use anyhow::Result;

use super::{Concurrency, EvaluationContext, ResolverContextLike};

pub trait Eval
where
  Self: Send + Sync,
{
  fn eval<'a, Ctx: ResolverContextLike<'a> + Sync + Send>(
    &'a self,
    ctx: &'a EvaluationContext<'a, Ctx>,
    conc: &'a Concurrency,
  ) -> Pin<Box<dyn Future<Output = Result<async_graphql::Value>> + 'a + Send>> {
    Box::pin(self.async_eval(ctx, conc))
  }

  fn async_eval<'a, Ctx: ResolverContextLike<'a> + Sync + Send>(
    &'a self,
    ctx: &'a EvaluationContext<'a, Ctx>,
    conc: &'a Concurrency,
  ) -> impl Future<Output = Result<async_graphql::Value>> + Send;
}
