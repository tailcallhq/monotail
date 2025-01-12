use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use async_graphql::parser::types::{
    Directive, DocumentOperations, ExecutableDocument, FragmentDefinition, OperationDefinition,
    OperationType, Selection, SelectionSet,
};
use async_graphql::Positioned;
use async_graphql_value::Value;

use super::model::{Directive as JitDirective, *};
use super::BuildError;
use crate::core::blueprint::{Blueprint, Index, QueryField};
use crate::core::counter::{Count, Counter};
use crate::core::jit::model::OperationPlan;
use crate::core::{scalar, Type};

#[derive(PartialEq, strum_macros::Display)]
enum Condition {
    True,
    False,
    Variable(Variable),
}

struct Conditions {
    skip: Condition,
    include: Condition,
}

impl Conditions {
    /// Checks if the field should be skipped always
    fn is_const_skip(&self) -> bool {
        // Truth Table
        // skip | include | ignore
        // T   |    T    |   T
        // T   |    F    |   T
        // F   |    T    |   F
        // F   |    F    |   T
        //
        // Logical expression:
        //     say skip = p, include = q
        // (p V ~q)

        // so instead of a normalizing variables,
        // we can just check for the above condition

        matches!(
            (&self.skip, &self.include),
            (Condition::True, _) | (Condition::False, Condition::False)
        )
    }

    fn into_variable_tuple(self) -> (Option<Variable>, Option<Variable>) {
        let comp = |condition| match condition {
            Condition::Variable(var) => Some(var),
            _ => None,
        };

        let include = comp(self.include);
        let skip = comp(self.skip);

        (include, skip)
    }
}

pub struct Builder<'a> {
    pub index: Arc<Index>,
    pub arg_id: Counter<usize>,
    pub field_id: Counter<usize>,
    pub document: &'a ExecutableDocument,
}

// TODO: make generic over Value (Input) type
impl<'a> Builder<'a> {
    pub fn new(blueprint: &Blueprint, document: &'a ExecutableDocument) -> Self {
        let index = Arc::new(blueprint.index());

        Self {
            document,
            index,
            arg_id: Counter::default(),
            field_id: Counter::default(),
        }
    }

    #[inline(always)]
    fn include(
        &self,
        directives: &[Positioned<async_graphql::parser::types::Directive>],
    ) -> Conditions {
        fn get_condition(dir: &Directive) -> Option<Condition> {
            let arg = dir.get_argument("if").map(|pos| &pos.node);
            match arg {
                None => None,
                Some(value) => match value {
                    Value::Boolean(bool) => {
                        let condition = if *bool {
                            Condition::True
                        } else {
                            Condition::False
                        };
                        Some(condition)
                    }
                    Value::Variable(var) => {
                        Some(Condition::Variable(Variable::new(var.deref().to_owned())))
                    }
                    _ => None,
                },
            }
        }
        Conditions {
            skip: directives
                .iter()
                .find(|d| d.node.name.node.as_str() == "skip")
                .map(|d| &d.node)
                .and_then(get_condition)
                .unwrap_or(Condition::False),
            include: directives
                .iter()
                .find(|d| d.node.name.node.as_str() == "include")
                .map(|d| &d.node)
                .and_then(get_condition)
                .unwrap_or(Condition::True),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn iter(
        &self,
        parent_fragment: Option<&str>,
        selection: &SelectionSet,
        type_condition: &str,
        fragments: &HashMap<&str, &FragmentDefinition>,
    ) -> Vec<Field<Value>> {
        let mut fields = vec![];

        for selection in &selection.items {
            match &selection.node {
                Selection::Field(Positioned { node: gql_field, .. }) => {
                    let conditions = self.include(&gql_field.directives);

                    // Skip fields based on GraphQL's skip/include conditions
                    if conditions.is_const_skip() {
                        continue;
                    }

                    let mut directives = Vec::with_capacity(gql_field.directives.len());
                    for directive in &gql_field.directives {
                        let directive = &directive.node;
                        if directive.name.node == "skip" || directive.name.node == "include" {
                            continue;
                        }
                        let arguments = directive
                            .arguments
                            .iter()
                            .map(|(k, v)| (k.node.to_string(), v.node.clone()))
                            .collect::<Vec<_>>();

                        directives
                            .push(JitDirective { name: directive.name.to_string(), arguments });
                    }

                    let (include, skip) = conditions.into_variable_tuple();
                    let field_name = gql_field.name.node.as_str();
                    let request_args = gql_field
                        .arguments
                        .iter()
                        .map(|(k, v)| (k.node.as_str().to_string(), v.node.to_owned()))
                        .collect::<HashMap<_, _>>();

                    let parent_fragment = parent_fragment.map(|s| s.to_owned());
                    // Check if the field is present in the schema index
                    if let Some(field_def) = self.index.get_field(type_condition, field_name) {
                        let mut args = Vec::with_capacity(request_args.len());
                        if let QueryField::Field((_, schema_args)) = field_def {
                            for (arg_name, arg_value) in schema_args {
                                let type_of = arg_value.of_type.clone();
                                let id = ArgId::new(self.arg_id.next());
                                let name = arg_name.clone();
                                let default_value = arg_value
                                    .default_value
                                    .as_ref()
                                    .and_then(|v| v.to_owned().try_into().ok());
                                args.push(Arg {
                                    id,
                                    name,
                                    type_of,
                                    value: request_args.get(arg_name).cloned(),
                                    default_value,
                                });
                            }
                        }

                        let type_of = match field_def {
                            QueryField::Field((field_def, _)) => field_def.of_type.clone(),
                            QueryField::InputField(field_def) => field_def.of_type.clone(),
                        };

                        let id = FieldId::new(self.field_id.next());

                        // Recursively gather child fields for the selection set
                        let child_fields = self.iter(
                            None,
                            &gql_field.selection_set.node,
                            type_of.name(),
                            fragments,
                        );

                        let ir = match field_def {
                            QueryField::Field((field_def, _)) => field_def.resolver.clone(),
                            _ => None,
                        };

                        let scalar = if self.index.type_is_scalar(type_of.name()) {
                            Some(
                                scalar::Scalar::find(type_of.name())
                                    .cloned()
                                    .unwrap_or(scalar::Scalar::Empty),
                            )
                        } else {
                            None
                        };

                        // Create the field with its child fields in `selection`
                        let field = Field {
                            id,
                            selection: child_fields,
                            parent_fragment,
                            name: field_name.to_string(),
                            output_name: gql_field
                                .alias
                                .as_ref()
                                .map(|a| a.node.to_string())
                                .unwrap_or(field_name.to_owned()),
                            ir,
                            is_enum: self.index.type_is_enum(type_of.name()),
                            type_of,
                            type_condition: Some(type_condition.to_string()),
                            skip,
                            include,
                            args,
                            pos: selection.pos.into(),
                            directives,
                            scalar,
                        };

                        fields.push(field);
                    } else if field_name == "__typename" {
                        let typename_field = Field {
                            id: FieldId::new(self.field_id.next()),
                            name: field_name.to_string(),
                            output_name: field_name.to_string(),
                            ir: None,
                            type_of: Type::Named { name: "String".to_owned(), non_null: true },
                            type_condition: None,
                            skip,
                            include,
                            args: Vec::new(),
                            pos: selection.pos.into(),
                            selection: vec![], // __typename has no child selection
                            parent_fragment,
                            directives,
                            is_enum: false,
                            scalar: Some(scalar::Scalar::Empty),
                        };

                        fields.push(typename_field);
                    }
                }
                Selection::FragmentSpread(Positioned { node: fragment_spread, .. }) => {
                    if let Some(fragment) =
                        fragments.get(fragment_spread.fragment_name.node.as_str())
                    {
                        fields.extend(self.iter(
                            Some(fragment.type_condition.node.on.node.as_str()),
                            &fragment.selection_set.node,
                            fragment.type_condition.node.on.node.as_str(),
                            fragments,
                        ));
                    }
                }
                Selection::InlineFragment(Positioned { node: fragment, .. }) => {
                    let type_of = fragment
                        .type_condition
                        .as_ref()
                        .map(|cond| cond.node.on.node.as_str())
                        .unwrap_or(type_condition);
                    fields.extend(self.iter(
                        Some(type_of),
                        &fragment.selection_set.node,
                        type_of,
                        fragments,
                    ));
                }
            }
        }

        fields
    }
    #[inline(always)]
    fn get_type(&self, ty: OperationType) -> Option<&str> {
        match ty {
            OperationType::Query => Some(self.index.get_query()),
            OperationType::Mutation => self.index.get_mutation(),
            OperationType::Subscription => None,
        }
    }

    /// Resolves currently processed operation
    /// based on [spec](https://spec.graphql.org/October2021/#sec-Executing-Requests)
    #[inline(always)]
    fn get_operation(
        &self,
        operation_name: Option<&str>,
    ) -> Result<&OperationDefinition, BuildError> {
        if let Some(operation_name) = operation_name {
            match &self.document.operations {
                DocumentOperations::Single(_) => None,
                DocumentOperations::Multiple(operations) => {
                    operations.get(operation_name).map(|op| &op.node)
                }
            }
            .ok_or_else(|| BuildError::OperationNotFound(operation_name.to_string()))
        } else {
            match &self.document.operations {
                DocumentOperations::Single(operation) => Ok(&operation.node),
                DocumentOperations::Multiple(map) if map.len() == 1 => {
                    let (_, operation) = map.iter().next().unwrap();
                    Ok(&operation.node)
                }
                DocumentOperations::Multiple(_) => Err(BuildError::OperationNameRequired),
            }
        }
    }

    #[inline(always)]
    pub fn build(&self, operation_name: Option<&str>) -> Result<OperationPlan<Value>, BuildError> {
        let mut fragments: HashMap<&str, &FragmentDefinition> = HashMap::new();

        for (name, fragment) in self.document.fragments.iter() {
            fragments.insert(name.as_str(), &fragment.node);
        }

        let operation = self.get_operation(operation_name)?;

        let name = self
            .get_type(operation.ty)
            .ok_or(BuildError::RootOperationTypeNotDefined { operation: operation.ty })?;
        let fields = self.iter(None, &operation.selection_set.node, name, &fragments);

        let is_introspection_query = operation.selection_set.node.items.iter().any(|f| {
            if let Selection::Field(Positioned { node: gql_field, .. }) = &f.node {
                let query = gql_field.name.node.as_str();
                query.contains("__schema") || query.contains("__type")
            } else {
                false
            }
        });

        let plan = OperationPlan::new(
            name,
            fields,
            operation.ty,
            self.index.clone(),
            is_introspection_query,
            Some(self.index.get_interfaces()),
        );
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tailcall_valid::Validator;

    use super::*;
    use crate::core::blueprint::Blueprint;
    use crate::core::config::Config;
    use crate::core::jit::builder::Builder;

    const CONFIG: &str = include_str!("./fixtures/jsonplaceholder-mutation.graphql");

    fn plan(query: impl AsRef<str>) -> OperationPlan<Value> {
        let config = Config::from_sdl(CONFIG).to_result().unwrap();
        let blueprint = Blueprint::try_from(&config.into()).unwrap();
        let document = async_graphql::parser::parse_query(query).unwrap();
        Builder::new(&blueprint, &document).build(None).unwrap()
    }

    #[tokio::test]
    async fn test_from_document() {
        let plan = plan(
            r#"
            query {
                posts { user { id name } }
            }
        "#,
        );
        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[tokio::test]
    async fn test_size() {
        let plan = plan(
            r#"
            query {
                posts { user { id name } }
            }
        "#,
        );

        assert!(plan.is_query());
        assert_eq!(plan.size(), 4)
    }

    #[test]
    fn test_simple_query() {
        let plan = plan(
            r#"
            query {
                posts { user { id } }
            }
        "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_alias_query() {
        let plan = plan(
            r#"
            query {
                articles: posts { author: user { identifier: id } }
            }
        "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_simple_mutation() {
        let plan = plan(
            r#"
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
        "#,
        );

        assert!(!plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_fragments() {
        let plan = plan(
            r#"
            fragment UserPII on User {
              name
              email
              phone
            }

            fragment PostPII on Post {
              title
              body
            }

            query {
              user(id:1) {
                ...UserPII
                ...PostPII
              }
            }
        "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_multiple_operations() {
        let plan = plan(
            r#"
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
        "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_variables() {
        let plan = plan(
            r#"
            query user($id: Int!) {
              user(id: $id) {
                id
                name
              }
            }
        "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_unions() {
        let plan = plan(
            r#"
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
        "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_default_value() {
        let plan = plan(
            r#"
            mutation {
              createPost(post:{
                userId:123,
                title:"tailcall",
                body:"tailcall test"
              }) {
                id
              }
            }
        "#,
        );

        assert!(!plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_condition() {
        // cases:
        // skip | include | ignore
        // T  |    T    |   T
        // T  |    F    |   T
        // T  |    V    |   T
        // F  |    F    |   T
        // F  |    T    |   F
        // F  |    V    |   F
        // V  |    T    |   F
        // V  |    F    |   F

        let test_var = Variable::new("ssdd.dev".to_string());

        let test_cases = vec![
            // ignore
            (Condition::True, Condition::True, true),
            (Condition::True, Condition::False, true),
            (Condition::True, Condition::Variable(test_var.clone()), true),
            (Condition::False, Condition::False, true),
            // don't ignore
            (Condition::False, Condition::True, false),
            (
                Condition::False,
                Condition::Variable(test_var.clone()),
                false,
            ),
            (
                Condition::Variable(test_var.clone()),
                Condition::True,
                false,
            ),
            (Condition::Variable(test_var), Condition::False, false),
        ];

        for (skip, include, expected) in test_cases {
            let conditions = Conditions { skip, include };
            assert_eq!(
                conditions.is_const_skip(),
                expected,
                "Failed for skip: {}, include: {}",
                conditions.skip,
                conditions.include
            );
        }
    }

    #[test]
    fn test_resolving_operation() {
        let query = r#"
            query GetPosts {
                posts {
                    id
                    userId
                    title
                }
            }

            mutation CreateNewPost {
                createPost(post: {
                    userId: 1,
                    title: "test-12",
                    body: "test-12",
                }) {
                    id
                    userId
                    title
                    body
                }
            }
        "#;
        let config = Config::from_sdl(CONFIG).to_result().unwrap();
        let blueprint = Blueprint::try_from(&config.into()).unwrap();
        let document = async_graphql::parser::parse_query(query).unwrap();
        let error = Builder::new(&blueprint, &document).build(None).unwrap_err();

        assert_eq!(error, BuildError::OperationNameRequired);

        let error = Builder::new(&blueprint, &document)
            .build(Some("unknown"))
            .unwrap_err();

        assert_eq!(error, BuildError::OperationNotFound("unknown".to_string()));

        let plan = Builder::new(&blueprint, &document)
            .build(Some("GetPosts"))
            .unwrap();
        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);

        let plan = Builder::new(&blueprint, &document)
            .build(Some("CreateNewPost"))
            .unwrap();
        assert!(!plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }

    #[test]
    fn test_directives() {
        let plan = plan(
            r#"
            query($includeName: Boolean! = true) {
                users {
                    id @options(paging: $includeName)
                    name @include(if: $includeName)
                }
            }
            "#,
        );

        assert!(plan.is_query());
        insta::assert_debug_snapshot!(plan.selection);
    }
}
