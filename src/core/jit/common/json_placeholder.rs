use std::collections::HashMap;
use std::rc::Rc;

use async_graphql_value::ConstValue;
use serde::Deserialize;
use serde_json_borrow::Value as BorrowedValue;

use crate::core::blueprint::Blueprint;
use crate::core::config::{Config, ConfigModule};
use crate::core::jit;
use crate::core::jit::builder::Builder;
use crate::core::jit::model::{ExecutionPlan, FieldId};
use crate::core::jit::store::{Data, Store};
use crate::core::jit::synth::Synth;
use crate::core::jit::SynthBorrow;
use crate::core::json::{JsonLike, JsonObjectLike};
use crate::core::valid::Validator;

/// NOTE: This is a bit of a boilerplate reducing module that is used in tests
/// and benchmarks.
pub struct JsonPlaceholder;

pub trait SynthExt<Value: JsonLike<Output = Value>> {
    fn init(plan: ExecutionPlan, data: Vec<(FieldId, Data<Value>)>) -> Self;
    fn synthesize(&'static self) -> jit::Result<Value>;
}

impl SynthExt<ConstValue> for Synth {
    fn init(plan: ExecutionPlan, data: Vec<(FieldId, Data<ConstValue>)>) -> Self {
        let store = data
            .into_iter()
            .fold(Store::new(), |mut store, (id, data)| {
                store.set_data(id, data.map(Ok));
                store
            });

        Synth::new(plan, store)
    }

    fn synthesize(&'static self) -> jit::Result<ConstValue> {
        self.synthesize()
    }
}
impl SynthExt<serde_json_borrow::Value<'static>> for SynthBorrow<'static> {
    fn init(plan: ExecutionPlan, data: Vec<(FieldId, Data<BorrowedValue<'static>>)>) -> Self {
        let store = data
            .into_iter()
            .fold(Store::new(), |mut store, (id, data)| {
                store.set_data(id, data);
                store
            });
        SynthBorrow::new(plan, store)
    }

    fn synthesize(&'static self) -> jit::Result<BorrowedValue<'static>> {
        Ok(self.synthesize())
    }
}

struct TestData<T> {
    posts: Vec<T>,
    users: HashMap<usize, Data<T>>,
}

impl JsonPlaceholder {
    const POSTS: &'static str = include_str!("posts.json");
    const USERS: &'static str = include_str!("users.json");
    const CONFIG: &'static str = include_str!("../fixtures/jsonplaceholder-mutation.graphql");

    fn value<'a, Value: JsonLike + Deserialize<'a> + Clone + 'static>() -> TestData<Value> {
        let posts = serde_json::from_str::<Vec<Value>>(Self::POSTS).unwrap();
        let users = serde_json::from_str::<Vec<Value>>(Self::USERS).unwrap();
        let user_map = users.iter().fold(HashMap::new(), |mut map, user| {
            let id = user
                .as_object_ok()
                .ok()
                .and_then(|user| user.get("id"))
                .and_then(|u| u.as_u64_ok().ok());
            if let Some(id) = id {
                map.insert(id, user);
            }
            map
        });
        let users: HashMap<_, _> = posts
            .iter()
            .map(|post| {
                let user_id = post
                    .as_object_ok()
                    .ok()
                    .and_then(|post| post.get("userId"))
                    .and_then(|u| u.as_u64_ok().ok());

                if let Some(user_id) = user_id {
                    if let Some(user) = user_map.get(&user_id) {
                        (*user).to_owned()
                    } else {
                        <Value as JsonLike>::default()
                    }
                } else {
                    <Value as JsonLike>::default()
                }
            })
            .map(Data::Single)
            .enumerate()
            .collect();

        TestData { posts, users }
    }

    fn plan(query: &str) -> ExecutionPlan {
        let config = ConfigModule::from(Config::from_sdl(Self::CONFIG).to_result().unwrap());
        let builder = Builder::new(
            &Blueprint::try_from(&config).unwrap(),
            async_graphql::parser::parse_query(query).unwrap(),
        );
        builder.build().unwrap()
    }

    fn data<'a, Value: JsonLike<Output = Value> + Deserialize<'a> + Clone + 'static>(
        plan: &ExecutionPlan,
        data: TestData<Value>,
    ) -> Vec<(FieldId, Data<Value>)> {
        let TestData { posts, users } = data;

        let posts_id = plan.find_field_path(&["posts"]).unwrap().id.to_owned();
        let users_id = plan
            .find_field_path(&["posts", "user"])
            .unwrap()
            .id
            .to_owned();
        let store = [
            (
                posts_id,
                Data::Single(<Value as JsonLike>::new_array(posts)),
            ),
            (users_id, Data::Multiple(users)),
        ];

        store.to_vec()
    }

    pub fn init<
        'a,
        Value: JsonLike<Output = Value> + Deserialize<'a> + Clone + 'static,
        T: SynthExt<Value>,
    >(
        query: &str,
    ) -> Rc<T> {
        let plan = Self::plan(query);
        let data = Self::value::<Value>();
        let data = Self::data(&plan, data);
        Rc::new(T::init(plan, data))
    }
}
