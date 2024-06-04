///
/// We need three executors for each query
/// 1. Global general purpose executor (WE have this currently)
/// 2. Query specific executor - optimized for each query
/// 4. ?? which is working a bit level
/// 5. Based on Data incoming and outgoing certain optimizations can further be
///    made.
mod field_index;
mod model {
    use std::collections::HashMap;
    use std::fmt::{Debug, Formatter};

    use async_graphql::parser::types::{DocumentOperations, ExecutableDocument, Selection};
    use async_graphql::Positioned;
    use async_graphql_parser::types::OperationType;

    use super::field_index::{FieldIndex, QueryField};
    use crate::core::blueprint::Blueprint;
    use crate::core::ir::IR;
    use crate::core::merge_right::MergeRight;

    #[allow(unused)]
    trait IncrGen {
        fn gen(&mut self) -> Self;
    }

    #[allow(unused)]
    #[derive(Debug)]
    pub enum Type {
        Named(String),
        List(Box<Type>),
        Required(Box<Type>),
    }

    #[allow(unused)]
    #[derive(Debug)]
    pub struct Arg {
        pub id: ArgId,
        pub name: String,
        pub type_of: crate::core::blueprint::Type,
        pub value: Option<async_graphql_value::Value>,
        pub default_value: Option<async_graphql_value::ConstValue>,
    }

    pub struct ArgId(usize);

    impl Debug for ArgId {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl IncrGen for ArgId {
        fn gen(&mut self) -> Self {
            let id = self.0;
            self.0 += 1;
            Self(id)
        }
    }

    #[allow(unused)]
    impl ArgId {
        fn new(id: usize) -> Self {
            ArgId(id)
        }
    }

    #[derive(Clone, PartialEq, Eq)]
    pub struct FieldId(usize);

    impl Debug for FieldId {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    #[allow(unused)]
    impl FieldId {
        fn new(id: usize) -> Self {
            FieldId(id)
        }
    }

    impl IncrGen for FieldId {
        fn gen(&mut self) -> Self {
            let id = self.0;
            self.0 += 1;
            Self(id)
        }
    }

    pub struct Field<A> {
        pub id: FieldId,
        pub name: String,
        pub ir: Option<IR>,
        pub type_of: crate::core::blueprint::Type,
        pub args: Vec<Arg>,
        pub refs: Option<A>,
    }

    impl<A: Debug + Clone> Debug for Field<A> {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            let mut debug_struct = f.debug_struct("Field");
            debug_struct.field("id", &self.id);
            debug_struct.field("name", &self.name);
            if self.ir.is_some() {
                debug_struct.field("ir", &"Some(..)");
            }
            debug_struct.field("type_of", &self.type_of);
            if !self.args.is_empty() {
                debug_struct.field("args", &self.args);
            }
            if self.refs.is_some() {
                debug_struct.field("refs", &self.refs);
            }
            debug_struct.finish()
        }
    }

    #[derive(Clone)]
    #[allow(unused)]
    pub struct Parent(FieldId);

    impl Debug for Parent {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "Parent({:?})", self.0)
        }
    }

    #[allow(unused)]
    pub struct Children(Vec<FieldId>);

    #[derive(Debug)]
    pub struct ExecutionPlan {
        pub fields: Vec<Field<Parent>>,
    }

    #[allow(unused)]
    pub struct ExecutionPlanBuilder {
        index: FieldIndex,
    }

    impl ExecutionPlanBuilder {
        #[allow(unused)]
        pub fn new(blueprint: Blueprint) -> Self {
            let blueprint_index = FieldIndex::init(&blueprint);
            Self { index: blueprint_index }
        }

        #[allow(unused)]
        pub fn build(&self, document: ExecutableDocument) -> anyhow::Result<ExecutionPlan> {
            let fields = self.create_field_set(document)?;
            Ok(ExecutionPlan { fields })
        }

        #[allow(clippy::too_many_arguments)]
        fn resolve_selection_set(
            &self,
            selection_set: Positioned<async_graphql_parser::types::SelectionSet>,
            id: &mut FieldId,
            arg_id: &mut ArgId,
            current_type: &str,
            parent: Option<Parent>,
        ) -> anyhow::Result<Vec<Field<Parent>>> {
            let mut fields = Vec::new();

            for selection in selection_set.node.items {
                if let Selection::Field(gql_field) = selection.node {
                    let field_name = gql_field.node.name.node.as_str();
                    let field_args = gql_field
                        .node
                        .arguments
                        .into_iter()
                        .map(|(k, v)| (k.node.as_str().to_string(), v.node))
                        .collect::<HashMap<_, _>>();

                    if let Some(field_def) = self.index.get_field(current_type, field_name) {
                        let mut args = vec![];
                        for (arg_name, value) in field_args {
                            if let Some(arg) = field_def.get_arg(&arg_name) {
                                let type_of = arg.of_type.clone();
                                let id = arg_id.gen();
                                let arg = Arg {
                                    id,
                                    name: arg_name.clone(),
                                    type_of,
                                    value: Some(value),
                                    default_value: arg
                                        .default_value
                                        .as_ref()
                                        .and_then(|v| v.to_owned().try_into().ok()),
                                };
                                args.push(arg);
                            }
                        }

                        let type_of = match field_def {
                            QueryField::Field((field_def, _)) => field_def.of_type.clone(),
                            QueryField::InputField(field_def) => field_def.of_type.clone(),
                        };

                        let cur_id = id.gen();
                        let child_fields = self.resolve_selection_set(
                            gql_field.node.selection_set.clone(),
                            id,
                            arg_id,
                            type_of.name(),
                            Some(Parent(cur_id.clone())),
                        )?;
                        let field = Field {
                            id: cur_id,
                            name: field_name.to_string(),
                            ir: match field_def {
                                QueryField::Field((field_def, _)) => field_def.resolver.clone(),
                                _ => None,
                            },
                            type_of,
                            args,
                            refs: parent.clone(),
                        };

                        fields.push(field);
                        fields = fields.merge_right(child_fields);
                    }
                }
            }

            Ok(fields)
        }

        fn create_field_set(
            &self,
            document: ExecutableDocument,
        ) -> anyhow::Result<Vec<Field<Parent>>> {
            let mut id = FieldId::new(0);
            let mut arg_id = ArgId::new(0);

            let mut fields = Vec::new();

            let operation_name_from_type = |operation_type| match operation_type {
                OperationType::Query => self.index.get_query(),
                OperationType::Mutation => self.index.get_mutation().unwrap(),
                OperationType::Subscription => unreachable!(),
            };

            for (_, fragment) in document.fragments {
                let current_type = fragment.node.type_condition.node.on.node.as_str();
                fields = self.resolve_selection_set(
                    fragment.node.selection_set,
                    &mut id,
                    &mut arg_id,
                    current_type,
                    None,
                )?;
            }

            match document.operations {
                DocumentOperations::Single(single) => {
                    let current_type = operation_name_from_type(single.node.ty);
                    fields = self.resolve_selection_set(
                        single.node.selection_set,
                        &mut id,
                        &mut arg_id,
                        current_type,
                        None,
                    )?;
                }
                DocumentOperations::Multiple(multiple) => {
                    for (_, single) in multiple {
                        let current_type = operation_name_from_type(single.node.ty);
                        fields = self.resolve_selection_set(
                            single.node.selection_set,
                            &mut id,
                            &mut arg_id,
                            current_type,
                            None,
                        )?;
                    }
                }
            }

            Ok(fields)
        }
    }
}

#[cfg(test)]
mod tests {
    use model::ExecutionPlan;

    use super::*;
    use crate::core::blueprint::Blueprint;
    use crate::core::config::Config;
    use crate::core::valid::Validator;

    const CONFIG: &str = include_str!("./fixtures/jsonplaceholder-mutation.graphql");

    fn create_query_plan(query: impl AsRef<str>) -> ExecutionPlan {
        let config = Config::from_sdl(CONFIG).to_result().unwrap();
        let blueprint = Blueprint::try_from(&config.into()).unwrap();
        let document = async_graphql::parser::parse_query(query).unwrap();

        model::ExecutionPlanBuilder::new(blueprint)
            .build(document)
            .unwrap()
    }

    #[test]
    fn test_simple_query() {
        let query = r#"
            query {
                posts { user { id } }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }

    #[test]
    fn test_simple_mutation() {
        let query = r#"
            mutation {
              createUser(user: {
                id: 101,
                name: "Tailcall",
                email: "tailcall@tailcall.run",
                phone: "2345234234",
                username: "tailcall",
                website: "tailcall.run"
              }) {
                id
                name
                email
                phone
                website
                username
              }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }

    #[test]
    fn test_fragments() {
        let query = r#"
            fragment UserPII on User {
              name
              email
              phone
            }

            query {
              user(id:1) {
                ...UserPII
              }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }

    #[test]
    fn test_multiple_operations() {
        let query = r#"
            query {
              user(id:1) {
                id
                username
              }
              posts {
                id
                title
              }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }

    #[test]
    fn test_variables() {
        let query = r#"
            query user($id: Int!) {
              user(id: $id) {
                id
                name
              }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }

    #[test]
    fn test_unions() {
        let query = r#"
            query {
              getUserIdOrEmail(id:1) {
                ...on UserId {
                  id
                }
                ...on UserEmail {
                  email
                }
              }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }

    #[test]
    fn test_default_value() {
        let query = r#"
            mutation {
              createPost(post:{
                userId:123,
                title:"tailcall",
                body:"tailcall test"
              }) {
                id
              }
            }
        "#;
        let plan = create_query_plan(query);
        insta::assert_debug_snapshot!(plan);
    }
}

mod value {
    pub use serde_json_borrow::*;
}

mod cache {
    use super::model::FieldId;
    use super::value::OwnedValue;

    #[allow(unused)]
    pub struct Cache {
        map: Vec<(FieldId, OwnedValue)>,
    }

    #[allow(unused)]
    impl Cache {
        #[allow(unused)]
        pub fn empty() -> Self {
            Cache { map: Vec::new() }
        }

        #[allow(unused)]
        pub fn join(caches: Vec<Cache>) -> Self {
            todo!()
        }
        #[allow(unused)]
        pub fn get(&self, key: FieldId) -> Option<&OwnedValue> {
            todo!()
        }
    }
}

mod executor {
    use futures_util::future;

    use super::cache::Cache;
    use super::model::{ExecutionPlan, Field, FieldId, Parent};
    use super::value::OwnedValue;
    use crate::core::ir::IR;

    #[allow(unused)]
    pub struct ExecutionContext {
        plan: ExecutionPlan,
        cache: Cache,
    }
    #[allow(unused)]
    impl ExecutionContext {
        pub async fn execute_ir(
            &self,
            ir: &IR,
            parent: Option<&OwnedValue>,
        ) -> anyhow::Result<OwnedValue> {
            todo!()
        }
        fn find_children(&self, id: FieldId) -> Vec<Field<Parent>> {
            todo!()
        }

        fn insert_field_value(&self, id: FieldId, value: OwnedValue) {
            todo!()
        }

        fn find_field(&self, id: FieldId) -> Option<&Field<Parent>> {
            self.plan.fields.iter().find(|field| field.id == id)
        }

        async fn execute_field(
            &self,
            id: FieldId,
            parent: Option<&OwnedValue>,
        ) -> anyhow::Result<()> {
            if let Some(field) = self.find_field(id.clone()) {
                if let Some(ir) = &field.ir {
                    let value = self.execute_ir(ir, parent).await?;

                    let children = self.find_children(id.clone());
                    future::join_all(
                        children
                            .into_iter()
                            .map(|child| self.execute_field(child.id, Some(&value))),
                    )
                    .await
                    .into_iter()
                    .collect::<anyhow::Result<Vec<_>>>()?;

                    self.insert_field_value(id, value);
                }
            }
            Ok(())
        }

        fn root(&self) -> Vec<&Field<Parent>> {
            self.plan
                .fields
                .iter()
                .filter(|field| field.refs.is_none())
                .collect::<Vec<_>>()
        }

        pub async fn execute(&self) -> anyhow::Result<()> {
            future::join_all(
                self.root()
                    .iter()
                    .map(|field| self.execute_field(field.id.to_owned(), None)),
            )
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(())
        }
    }
}

mod synth {
    pub use serde_json_borrow::*;

    use super::cache::Cache;
    use super::model::ExecutionPlan;

    struct Synth {
        blueprint: ExecutionPlan,
        cache: Cache,
    }
    #[allow(unused)]
    impl Synth {
        pub fn new(blueprint: ExecutionPlan) -> Self {
            Synth { blueprint, cache: Cache::empty() }
        }

        pub fn synthesize(&self) -> Value<'_> {
            let mut object = ObjectAsVec::default();

            let root_fields = self.blueprint.fields.iter().filter(|a| a.refs.is_none());

            for root_field in root_fields {
                let key = &root_field.name;
                let id = root_field.id.to_owned();
                if let Some(value) = self.cache.get(id) {
                    object.insert(key, value.get_value().to_owned());
                }
            }

            Value::Object(object)
        }
    }
}
