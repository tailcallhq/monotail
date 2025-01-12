use std::collections::{BTreeMap, HashSet};

use directive::to_directive;
use tailcall_valid::{Valid, Validator};

use crate::core::blueprint::*;
use crate::core::config::{Config, Field, Type};
use crate::core::directive::DirectiveCodec;

fn validate_query(config: &Config) -> Valid<&str, BlueprintError> {
    let query_type_name = config
        .schema
        .query
        .as_deref()
        // Based on the [spec](https://spec.graphql.org/October2021/#sec-Root-Operation-Types.Default-Root-Operation-Type-Names)
        // the default name for query type is `Query` is not specified explicitly
        .unwrap_or("Query");

    let Some(query) = config.find_type(query_type_name) else {
        // from spec: The query root operation type must be provided and must be an
        // Object type.
        return Valid::fail(BlueprintError::QueryTypeNotDefined).trace(query_type_name);
    };
    let mut set = HashSet::new();

    validate_type_has_resolvers(query_type_name, query, &config.types, &mut set)
        .map_to(query_type_name)
}

/// Validates that all the root type fields has resolver
/// making into the account the nesting
fn validate_type_has_resolvers(
    name: &str,
    ty: &Type,
    types: &BTreeMap<String, Type>,
    visited: &mut HashSet<String>,
) -> Valid<(), BlueprintError> {
    if ty.scalar() || visited.contains(name) {
        return Valid::succeed(());
    }

    visited.insert(name.to_string());

    Valid::from_iter(ty.fields.iter(), |(name, field)| {
        validate_field_has_resolver(name, field, types, visited)
    })
    .trace(name)
    .unit()
}

pub fn validate_field_has_resolver(
    name: &str,
    field: &Field,
    types: &BTreeMap<String, Type>,
    visited: &mut HashSet<String>,
) -> Valid<(), BlueprintError> {
    Valid::<(), BlueprintError>::fail(BlueprintError::NoResolverFoundInSchema)
        .when(|| {
            if !field.has_resolver() {
                let type_name = field.type_of.name();
                if let Some(ty) = types.get(type_name) {
                    let res = validate_type_has_resolvers(type_name, ty, types, visited);
                    return !res.is_succeed();
                }

                return true;
            }
            false
        })
        .trace(name)
}

fn validate_mutation(config: &Config) -> Valid<Option<&str>, BlueprintError> {
    let mutation_type_name = config
        .schema
        .mutation
        .as_deref()
        // Based on the [spec](https://spec.graphql.org/October2021/#sec-Root-Operation-Types.Default-Root-Operation-Type-Names)
        // the default name for mutation type is `Mutation` is not specified explicitly
        .unwrap_or("Mutation");

    if let Some(mutation) = config.find_type(mutation_type_name) {
        let mut set = HashSet::new();
        validate_type_has_resolvers(mutation_type_name, mutation, &config.types, &mut set)
            .map_to(Some(mutation_type_name))
    } else if config.schema.mutation.is_some() {
        // if mutation was specified by schema but not found raise the error
        Valid::fail(BlueprintError::MutationTypeNotDefined).trace(mutation_type_name)
    } else {
        // otherwise if mutation is not specified and default type is not found just
        // return None
        Valid::succeed(None)
    }
}

pub fn to_schema<'a>() -> TryFoldConfig<'a, SchemaDefinition> {
    TryFoldConfig::new(|config, _| {
        validate_query(config)
            .fuse(validate_mutation(config))
            .fuse(to_directive(config.server.to_directive()))
            .map(
                |(query_type_name, mutation_type_name, directive)| SchemaDefinition {
                    query: query_type_name.to_owned(),
                    mutation: mutation_type_name.map(|x| x.to_owned()),
                    directives: vec![directive],
                },
            )
    })
}
