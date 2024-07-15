use std::fmt::Debug;
use std::mem;
use std::sync::{Arc, Mutex};

use derive_getters::Getters;
use futures_util::future::join_all;

use super::context::Context;
use super::synth::Synthesizer;
use super::{DataPath, Field, Nested, OperationPlan, Request, Response, Store};
use crate::core::ir::model::IR;
use crate::core::json::JsonLike;

///
/// Default GraphQL executor that takes in a GraphQL Request and produces a
/// GraphQL Response
pub struct Executor<Synth, IRExec> {
    plan: OperationPlan,
    synth: Synth,
    exec: IRExec,
}

impl<Input: Clone, Output, Error, Synth, Exec> Executor<Synth, Exec>
where
    Output: JsonLike<Json = Output> + Debug,
    Synth: Synthesizer<Value = Result<Output, Error>, Variable = Input>,
    Exec: IRExecutor<Input = Input, Output = Output, Error = Error>,
{
    pub fn new(plan: OperationPlan, synth: Synth, exec: Exec) -> Self {
        Self { plan, synth, exec }
    }

    async fn execute_inner(&self, request: Request<Input>) -> Store<Result<Output, Error>> {
        let store: Arc<Mutex<Store<Result<Output, Error>>>> = Arc::new(Mutex::new(Store::new()));
        let mut ctx = ExecutorInner::new(request, store.clone(), self.plan.to_owned(), &self.exec);
        ctx.init().await;

        let store = mem::replace(&mut *store.lock().unwrap(), Store::new());
        store
    }

    pub async fn execute(self, request: Request<Input>) -> Response<Output, Error> {
        let vars = request.variables.clone();
        let store = self.execute_inner(request).await;
        Response::new(self.synth.synthesize(store, vars))
    }
}

#[derive(Getters)]
struct ExecutorInner<'a, Input, Output, Error, Exec> {
    request: Request<Input>,
    store: Arc<Mutex<Store<Result<Output, Error>>>>,
    plan: OperationPlan,
    ir_exec: &'a Exec,
}

impl<'a, Input, Output, Error, Exec> ExecutorInner<'a, Input, Output, Error, Exec>
where
    Output: JsonLike<Json = Output> + Debug,
    Exec: IRExecutor<Input = Input, Output = Output, Error = Error>,
{
    fn new(
        request: Request<Input>,
        store: Arc<Mutex<Store<Result<Output, Error>>>>,
        plan: OperationPlan,
        ir_exec: &'a Exec,
    ) -> Self {
        Self { request, store, plan, ir_exec }
    }

    async fn init(&mut self) {
        let ctx = Context::new(&self.request, self.plan.is_query());
        join_all(
            self.plan
                .as_nested()
                .iter()
                .map(|field| async { self.execute(field, &ctx, DataPath::new()).await }),
        )
        .await;
    }

    async fn execute<'b>(
        &'b self,
        field: &'b Field<Nested>,
        ctx: &'b Context<'b, Input, Output>,
        data_path: DataPath,
    ) -> Result<(), Error> {
        if let Some(ir) = &field.ir {
            let result = self.ir_exec.execute(ir, ctx).await;

            if let Ok(ref value) = result {
                // Array
                // Check if the field expects a list
                if field.type_of.is_list() {
                    // Check if the value is an array
                    if let Ok(array) = value.as_array_ok() {
                        join_all(field.nested().iter().map(|field| {
                            join_all(array.iter().enumerate().map(|(index, value)| {
                                let ctx = ctx.with_value(value);
                                let data_path = data_path.clone().with_index(index);
                                async move { self.execute(field, &ctx, data_path).await }
                            }))
                        }))
                        .await;
                    }
                    // TODO:  We should throw an error stating that we expected
                    // a list type here but because the `Error` is a
                    // type-parameter, its not possible
                }
                // TODO: Validate if the value is an Object
                // Has to be an Object, we don't do anything while executing if its a Scalar
                else {
                    join_all(field.nested().iter().map(|child| {
                        let ctx = ctx.with_value(value);
                        let data_path = data_path.clone();
                        async move { self.execute(child, &ctx, data_path).await }
                    }))
                    .await;
                }
            }

            let mut store = self.store.lock().unwrap();

            store.set(&field.id, &data_path, result);
        }

        Ok(())
    }
}

/// Executor for IR
#[async_trait::async_trait]
pub trait IRExecutor {
    type Input;
    type Output;
    type Error;
    async fn execute<'a>(
        &'a self,
        ir: &'a IR,
        ctx: &'a Context<'a, Self::Input, Self::Output>,
    ) -> Result<Self::Output, Self::Error>;
}
