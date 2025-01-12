use std::fmt::Display;

use async_graphql_value::{ConstValue, Value};

use super::super::{Arg, Field, OperationPlan, ResolveInputError, Variables};
use crate::core::blueprint::Index;
use crate::core::ir::model::IO;
use crate::core::json::{JsonLikeOwned, JsonObjectLike};
use crate::core::Type;

/// Trait to represent conversion from some dynamic type (with variables)
/// to the resolved variant based on the additional provided info.
/// E.g. conversion from [async_graphql_value::Value] ->
/// [async_graphql_value::ConstValue]
pub trait InputResolvable {
    type Output;

    fn resolve(
        self,
        variables: &Variables<Self::Output>,
    ) -> Result<Self::Output, ResolveInputError>;
}

impl InputResolvable for Value {
    type Output = ConstValue;

    fn resolve(self, variables: &Variables<ConstValue>) -> Result<Self::Output, ResolveInputError> {
        self.into_const_with(|name| {
            variables
                .get(&name)
                .cloned()
                .ok_or_else(|| ResolveInputError::VariableIsNotFound(name.to_string()))
        })
    }
}

/// Transforms [OperationPlan] values the way that all the input values
/// are transformed to const variant with the help of [InputResolvable] trait
pub struct InputResolver<Input> {
    plan: OperationPlan<Input>,
}

impl<Input> InputResolver<Input> {
    pub fn new(plan: OperationPlan<Input>) -> Self {
        Self { plan }
    }
}

impl<Input, Output> InputResolver<Input>
where
    Input: Clone + std::fmt::Debug,
    Output: Clone + JsonLikeOwned + TryFrom<serde_json::Value> + std::fmt::Debug + Display,
    Input: InputResolvable<Output = Output>,
    <Output as TryFrom<serde_json::Value>>::Error: std::fmt::Debug,
{
    pub fn resolve_input(
        self,
        variables: &Variables<Output>,
    ) -> Result<OperationPlan<Output>, ResolveInputError> {
        let index = self.plan.index;
        let mut selection = self
            .plan
            .selection
            .into_iter()
            .map(|field| field.try_map(&|value| value.resolve(variables)))
            // Call `resolve_field` to verify/populate defaults for args
            // because the previous map will just try convert values based on
            // variables ignoring default values in schema and not checking if arg
            // is required TODO: consider changing [Field::try_map] to be able to do
            // this check?
            .map(|field| Self::resolve_field(&index, field?))
            .collect::<Result<Vec<_>, _>>()?;

        // adjust the pre-computed values in selection set like graphql query for
        // @graphql directive.
        Self::resolve_graphql_selection_set(&mut selection, variables);

        Ok(OperationPlan {
            root_name: self.plan.root_name.to_string(),
            operation_type: self.plan.operation_type,
            index,
            is_introspection_query: self.plan.is_introspection_query,
            is_dedupe: self.plan.is_dedupe,
            is_const: self.plan.is_const,
            is_protected: self.plan.is_protected,
            min_cache_ttl: self.plan.min_cache_ttl,
            interfaces: None,
            selection,
            before: self.plan.before,
        })
    }

    // resolves the variables in selection set mustache template for graphql query.
    fn resolve_graphql_selection_set(
        base_field: &mut [Field<Output>],
        variables: &Variables<Output>,
    ) {
        for field in base_field.iter_mut() {
            if let Some(ir) = field.ir.as_mut() {
                ir.modify_io(&mut |io| {
                    if let IO::GraphQL { req_template, .. } = io {
                        if let Some(selection) = req_template.selection.take() {
                            req_template.selection = Some(selection.resolve(variables));
                        }
                    }
                });
            }
            Self::resolve_graphql_selection_set(field.selection.as_mut(), variables);
        }
    }

    fn resolve_field(
        index: &Index,
        field: Field<Output>,
    ) -> Result<Field<Output>, ResolveInputError> {
        // TODO: should also check and provide defaults for directives
        let args = field
            .args
            .into_iter()
            .map(|arg| {
                let value = Self::recursive_parse_arg(
                    index,
                    &field.name,
                    &arg.name,
                    &arg.type_of,
                    &arg.default_value,
                    arg.value,
                )?;
                Ok(Arg { value, ..arg })
            })
            .collect::<Result<_, _>>()?;

        let selection = field
            .selection
            .into_iter()
            .map(|field| Self::resolve_field(index, field))
            .collect::<Result<_, _>>()?;

        Ok(Field { args, selection, ..field })
    }

    #[allow(clippy::too_many_arguments)]
    fn recursive_parse_arg(
        index: &Index,
        parent_name: &str,
        arg_name: &str,
        type_of: &Type,
        default_value: &Option<Output>,
        value: Option<Output>,
    ) -> Result<Option<Output>, ResolveInputError> {
        let is_value_null = value.as_ref().map(|val| val.is_null()).unwrap_or(true);
        let value = if !type_of.is_nullable() && value.is_none() {
            let default_value = default_value.clone();

            Some(default_value.ok_or(ResolveInputError::ArgumentIsRequired {
                arg_name: arg_name.to_string(),
                field_name: parent_name.to_string(),
            })?)
        } else if !type_of.is_nullable() && is_value_null {
            return Err(ResolveInputError::ArgumentIsRequired {
                arg_name: arg_name.to_string(),
                field_name: parent_name.to_string(),
            });
        } else if value.is_none() {
            default_value.clone()
        } else {
            value
        };

        let Some(mut value) = value else {
            return Ok(None);
        };

        let Some(def) = index.get_input_type_definition(type_of.name()) else {
            return Ok(Some(value));
        };

        if let Some(obj) = value.as_object_mut() {
            for arg_field in &def.fields {
                let parent_name = format!("{}.{}", parent_name, arg_name);
                let field_value = obj.get_key(&arg_field.name).cloned();
                let field_default = arg_field
                    .default_value
                    .clone()
                    .map(|value| Output::try_from(value).expect("The conversion cannot fail"));
                let value = Self::recursive_parse_arg(
                    index,
                    &parent_name,
                    &arg_field.name,
                    &arg_field.of_type,
                    &field_default,
                    field_value,
                )?;
                if let Some(value) = value {
                    obj.insert_key(&arg_field.name, value);
                }
            }
        } else if let Some(arr) = value.as_array_mut() {
            for (i, item) in arr.iter_mut().enumerate() {
                let parent_name = format!("{}.{}.{}", parent_name, arg_name, i);

                *item = Self::recursive_parse_arg(
                    index,
                    &parent_name,
                    &i.to_string(),
                    type_of,
                    &None,
                    Some(item.clone()),
                )?
                .expect("Because we start with `Some`, we will end with `Some`");
            }
        }

        Ok(Some(value))
    }
}
