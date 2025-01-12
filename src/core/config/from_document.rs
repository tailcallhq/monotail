use std::collections::{BTreeMap, BTreeSet};

use async_graphql::parser::types::{
    ConstDirective, EnumType, FieldDefinition, InputObjectType, InputValueDefinition,
    InterfaceType, ObjectType, SchemaDefinition, ServiceDocument, Type, TypeDefinition, TypeKind,
    TypeSystemDefinition, UnionType,
};
use async_graphql::parser::Positioned;
use async_graphql::Name;
use async_graphql_value::ConstValue;
use indexmap::IndexMap;
use tailcall_valid::{Valid, ValidationError, Validator};

use super::directive::{to_directive, Directive};
use super::{Alias, Discriminate, Resolver, RuntimeConfig, Telemetry, FEDERATION_DIRECTIVES};
use crate::core::config::{
    self, Cache, Config, Enum, Link, Modify, Omit, Protected, RootSchema, Server, Union, Upstream,
    Variant,
};
use crate::core::directive::DirectiveCodec;

const DEFAULT_SCHEMA_DEFINITION: &SchemaDefinition = &SchemaDefinition {
    extend: false,
    directives: Vec::new(),
    query: None,
    mutation: None,
    subscription: None,
};

pub fn from_document(doc: ServiceDocument) -> Valid<Config, String> {
    let type_definitions: Vec<_> = doc
        .definitions
        .iter()
        .filter_map(|def| match def {
            TypeSystemDefinition::Type(td) => Some(td),
            _ => None,
        })
        .collect();

    let types = to_types(&type_definitions);
    let unions = to_union_types(&type_definitions);
    let enums = to_enum_types(&type_definitions);
    let schema = schema_definition(&doc).map(to_root_schema);
    schema_definition(&doc).and_then(|sd| {
        server(sd)
            .fuse(upstream(sd))
            .fuse(types)
            .fuse(unions)
            .fuse(enums)
            .fuse(schema)
            .fuse(links(sd))
            .fuse(telemetry(sd))
            .map(
                |(server, upstream, types, unions, enums, schema, links, telemetry)| {
                    let runtime_config = RuntimeConfig { server, upstream, links, telemetry };
                    let config = Config { types, unions, enums, schema, ..Default::default() };

                    config.with_runtime_config(runtime_config)
                },
            )
    })
}

fn schema_definition(doc: &ServiceDocument) -> Valid<&SchemaDefinition, String> {
    doc.definitions
        .iter()
        .find_map(|def| match def {
            TypeSystemDefinition::Schema(schema_definition) => Some(&schema_definition.node),
            _ => None,
        })
        .map_or_else(|| Valid::succeed(DEFAULT_SCHEMA_DEFINITION), Valid::succeed)
}

fn process_schema_directives<T: DirectiveCodec + Default>(
    schema_definition: &SchemaDefinition,
    directive_name: &str,
) -> Valid<T, String> {
    let mut res = Valid::succeed(T::default());
    for directive in schema_definition.directives.iter() {
        if directive.node.name.node.as_ref() == directive_name {
            tracing::warn!("@{} directive definition in the graphql schema file is deprecated. Please, refer to blog post https://tailcall.run/blog/migrating-to-graphql-configuration-v2/", directive_name);
            res = T::from_directive(&directive.node);
        }
    }
    res
}

fn process_schema_multiple_directives<T: DirectiveCodec + Default>(
    schema_definition: &SchemaDefinition,
    directive_name: &str,
) -> Valid<Vec<T>, String> {
    let directives: Vec<Valid<T, String>> = schema_definition
        .directives
        .iter()
        .filter_map(|directive| {
            if directive.node.name.node.as_ref() == directive_name {
                Some(T::from_directive(&directive.node))
            } else {
                None
            }
        })
        .collect();

    Valid::from_iter(directives, |item| item)
}

fn server(schema_definition: &SchemaDefinition) -> Valid<Server, String> {
    process_schema_directives(schema_definition, config::Server::directive_name().as_str())
}

fn upstream(schema_definition: &SchemaDefinition) -> Valid<Upstream, String> {
    process_schema_directives(
        schema_definition,
        config::Upstream::directive_name().as_str(),
    )
}

fn links(schema_definition: &SchemaDefinition) -> Valid<Vec<Link>, String> {
    process_schema_multiple_directives(schema_definition, config::Link::directive_name().as_str())
}

fn telemetry(schema_definition: &SchemaDefinition) -> Valid<Telemetry, String> {
    process_schema_directives(
        schema_definition,
        super::Telemetry::directive_name().as_str(),
    )
}

fn to_root_schema(schema_definition: &SchemaDefinition) -> RootSchema {
    let query = schema_definition.query.as_ref().map(pos_name_to_string);
    let mutation = schema_definition.mutation.as_ref().map(pos_name_to_string);
    let subscription = schema_definition
        .subscription
        .as_ref()
        .map(pos_name_to_string);

    RootSchema { query, mutation, subscription }
}
fn pos_name_to_string(pos: &Positioned<Name>) -> String {
    pos.node.to_string()
}
fn to_types(
    type_definitions: &Vec<&Positioned<TypeDefinition>>,
) -> Valid<BTreeMap<String, config::Type>, String> {
    Valid::from_iter(type_definitions, |type_definition| {
        let type_name = pos_name_to_string(&type_definition.node.name);
        match type_definition.node.kind.clone() {
            TypeKind::Object(object_type) => to_object_type(
                &object_type,
                &type_definition.node.description,
                &type_definition.node.directives,
            )
            .trace(&type_name)
            .some(),
            TypeKind::Interface(interface_type) => to_object_type(
                &interface_type,
                &type_definition.node.description,
                &type_definition.node.directives,
            )
            .trace(&type_name)
            .some(),
            TypeKind::Enum(_) => Valid::none(),
            TypeKind::InputObject(input_object_type) => to_input_object(
                input_object_type,
                &type_definition.node.description,
                &type_definition.node.directives,
            )
            .trace(&type_name)
            .some(),
            TypeKind::Union(_) => Valid::none(),
            TypeKind::Scalar => Valid::succeed(Some(to_scalar_type())),
        }
        .map(|option| (type_name, option))
    })
    .map(|vec| {
        BTreeMap::from_iter(
            vec.into_iter()
                .filter_map(|(name, option)| option.map(|tpe| (name, tpe))),
        )
    })
}
fn to_scalar_type() -> config::Type {
    config::Type { ..Default::default() }
}
fn to_union_types(
    type_definitions: &[&Positioned<TypeDefinition>],
) -> Valid<BTreeMap<String, Union>, String> {
    Valid::from_iter(type_definitions.iter(), |type_definition| {
        let type_name = pos_name_to_string(&type_definition.node.name);
        let type_opt = match type_definition.node.kind.clone() {
            TypeKind::Union(union_type) => to_union(
                union_type,
                &type_definition
                    .node
                    .description
                    .to_owned()
                    .map(|pos| pos.node),
            ),
            _ => return Valid::succeed(None),
        };
        type_opt.map(|type_opt| Some((type_name, type_opt)))
    })
    .map(|values| values.into_iter().flatten().collect())
}

fn to_enum_types(
    type_definitions: &[&Positioned<TypeDefinition>],
) -> Valid<BTreeMap<String, Enum>, String> {
    Valid::from_iter(type_definitions.iter(), |type_definition| {
        let type_name = pos_name_to_string(&type_definition.node.name);
        let type_opt = match type_definition.node.kind.clone() {
            TypeKind::Enum(enum_type) => to_enum(
                enum_type,
                type_definition
                    .node
                    .description
                    .to_owned()
                    .map(|pos| pos.node),
            ),
            _ => return Valid::succeed(None),
        };
        type_opt.map(|type_opt| Some((type_name, type_opt)))
    })
    .map(|values| values.into_iter().flatten().collect())
}

fn to_object_type<T>(
    object: &T,
    description: &Option<Positioned<String>>,
    directives: &[Positioned<ConstDirective>],
) -> Valid<config::Type, String>
where
    T: ObjectLike,
{
    let fields = object.fields();
    let implements = object.implements();

    Resolver::from_directives(directives)
        .fuse(Cache::from_directives(directives.iter()))
        .fuse(to_fields(fields))
        .fuse(Protected::from_directives(directives.iter()))
        .fuse(to_add_fields_from_directives(directives))
        .fuse(to_federation_directives(directives))
        .map(
            |(resolvers, cache, fields, protected, added_fields, unknown_directives)| {
                let doc = description.to_owned().map(|pos| pos.node);
                let implements = implements.iter().map(|pos| pos.node.to_string()).collect();
                config::Type {
                    fields,
                    added_fields,
                    doc,
                    implements,
                    cache,
                    protected,
                    resolvers,
                    directives: unknown_directives,
                }
            },
        )
}
fn to_input_object(
    input_object_type: InputObjectType,
    description: &Option<Positioned<String>>,
    directives: &[Positioned<ConstDirective>],
) -> Valid<config::Type, String> {
    to_input_object_fields(&input_object_type.fields)
        .fuse(Protected::from_directives(directives.iter()))
        .map(|(fields, protected)| {
            let doc = description.to_owned().map(|pos| pos.node);
            config::Type { fields, protected, doc, ..Default::default() }
        })
}

fn to_fields_inner<T, F>(
    fields: &Vec<Positioned<T>>,
    transform: F,
) -> Valid<BTreeMap<String, config::Field>, String>
where
    F: Fn(&T) -> Valid<config::Field, String>,
    T: HasName,
{
    Valid::from_iter(fields, |field| {
        let field_name = pos_name_to_string(field.node.name());
        transform(&field.node).map(|field| (field_name, field))
    })
    .map(BTreeMap::from_iter)
}
fn to_fields(
    fields: &Vec<Positioned<FieldDefinition>>,
) -> Valid<BTreeMap<String, config::Field>, String> {
    to_fields_inner(fields, to_field)
}
fn to_input_object_fields(
    input_object_fields: &Vec<Positioned<InputValueDefinition>>,
) -> Valid<BTreeMap<String, config::Field>, String> {
    to_fields_inner(input_object_fields, to_input_object_field)
}
fn to_field(field_definition: &FieldDefinition) -> Valid<config::Field, String> {
    to_common_field(field_definition, to_args(field_definition), None)
}
fn to_input_object_field(field_definition: &InputValueDefinition) -> Valid<config::Field, String> {
    to_common_field(
        field_definition,
        IndexMap::new(),
        field_definition
            .default_value
            .as_ref()
            .map(|f| f.node.clone()),
    )
}
fn to_common_field<F>(
    field: &F,
    args: IndexMap<String, config::Arg>,
    default_value: Option<ConstValue>,
) -> Valid<config::Field, String>
where
    F: FieldLike + HasName,
{
    let type_of = field.type_of();
    let description = field.description();
    let directives = field.directives();
    let default_value = default_value
        .map(ConstValue::into_json)
        .transpose()
        .map_err(|err| ValidationError::new(err.to_string()))
        .into();
    let doc = description.to_owned().map(|pos| pos.node);

    config::Resolver::from_directives(directives)
        .fuse(Cache::from_directives(directives.iter()))
        .fuse(Omit::from_directives(directives.iter()))
        .fuse(Modify::from_directives(directives.iter()))
        .fuse(Protected::from_directives(directives.iter()))
        .fuse(Discriminate::from_directives(directives.iter()))
        .fuse(default_value)
        .fuse(to_federation_directives(directives))
        .map(
            |(
                resolvers,
                cache,
                omit,
                modify,
                protected,
                discriminate,
                default_value,
                directives,
            )| config::Field {
                type_of: type_of.into(),
                args,
                doc,
                modify,
                omit,
                cache,
                protected,
                discriminate,
                default_value,
                resolvers,
                directives,
            },
        )
        .trace(pos_name_to_string(field.name()).as_str())
}

fn to_args(field_definition: &FieldDefinition) -> IndexMap<String, config::Arg> {
    let mut args = IndexMap::new();

    for arg in field_definition.arguments.iter() {
        let arg_name = pos_name_to_string(&arg.node.name);
        let arg_val = to_arg(&arg.node);
        args.insert(arg_name, arg_val);
    }

    args
}
fn to_arg(input_value_definition: &InputValueDefinition) -> config::Arg {
    let type_of = &input_value_definition.ty.node;
    let doc = input_value_definition
        .description
        .to_owned()
        .map(|pos| pos.node);
    let modify = Modify::from_directives(input_value_definition.directives.iter())
        .to_result()
        .ok()
        .flatten();
    let default_value = if let Some(pos) = input_value_definition.default_value.as_ref() {
        let value = &pos.node;
        serde_json::to_value(value).ok()
    } else {
        None
    };
    config::Arg { type_of: type_of.into(), doc, modify, default_value }
}

fn to_union(union_type: UnionType, doc: &Option<String>) -> Valid<Union, String> {
    let types = union_type
        .members
        .iter()
        .map(|member| member.node.to_string())
        .collect();

    Valid::succeed(Union { types, doc: doc.clone() })
}

fn to_enum(enum_type: EnumType, doc: Option<String>) -> Valid<Enum, String> {
    let variants = Valid::from_iter(enum_type.values.iter(), |member| {
        let name = member.node.value.node.as_str().to_owned();
        let alias = member
            .node
            .directives
            .iter()
            .find(|d| d.node.name.node.as_str() == Alias::directive_name());
        if let Some(alias) = alias {
            Alias::from_directive(&alias.node).map(|alias| Variant { name, alias: Some(alias) })
        } else {
            Valid::succeed(Variant { name, alias: None })
        }
    });
    variants.map(|v| Enum { variants: v.into_iter().collect::<BTreeSet<Variant>>(), doc })
}

fn to_add_fields_from_directives(
    directives: &[Positioned<ConstDirective>],
) -> Valid<Vec<config::AddField>, String> {
    Valid::from_iter(
        directives
            .iter()
            .filter(|v| v.node.name.node == config::AddField::directive_name()),
        |directive| {
            let val = config::AddField::from_directive(&directive.node).to_result();
            Valid::from(val)
        },
    )
}

fn to_federation_directives(
    directives: &[Positioned<ConstDirective>],
) -> Valid<Vec<Directive>, String> {
    Valid::from_iter(directives.iter(), |directive| {
        if FEDERATION_DIRECTIVES
            .iter()
            .any(|&known| known == directive.node.name.node.as_str())
        {
            to_directive(directive.node.clone()).map(Some)
        } else {
            Valid::succeed(None)
        }
    })
    .map(|directives| directives.into_iter().flatten().collect())
}

trait HasName {
    fn name(&self) -> &Positioned<Name>;
}
impl HasName for FieldDefinition {
    fn name(&self) -> &Positioned<Name> {
        &self.name
    }
}
impl HasName for InputValueDefinition {
    fn name(&self) -> &Positioned<Name> {
        &self.name
    }
}

trait FieldLike {
    fn type_of(&self) -> &Type;
    fn description(&self) -> &Option<Positioned<String>>;
    fn directives(&self) -> &[Positioned<ConstDirective>];
}
impl FieldLike for FieldDefinition {
    fn type_of(&self) -> &Type {
        &self.ty.node
    }
    fn description(&self) -> &Option<Positioned<String>> {
        &self.description
    }
    fn directives(&self) -> &[Positioned<ConstDirective>] {
        &self.directives
    }
}
impl FieldLike for InputValueDefinition {
    fn type_of(&self) -> &Type {
        &self.ty.node
    }
    fn description(&self) -> &Option<Positioned<String>> {
        &self.description
    }
    fn directives(&self) -> &[Positioned<ConstDirective>] {
        &self.directives
    }
}

trait ObjectLike {
    fn fields(&self) -> &Vec<Positioned<FieldDefinition>>;
    fn implements(&self) -> &Vec<Positioned<Name>>;
}
impl ObjectLike for ObjectType {
    fn fields(&self) -> &Vec<Positioned<FieldDefinition>> {
        &self.fields
    }
    fn implements(&self) -> &Vec<Positioned<Name>> {
        &self.implements
    }
}
impl ObjectLike for InterfaceType {
    fn fields(&self) -> &Vec<Positioned<FieldDefinition>> {
        &self.fields
    }
    fn implements(&self) -> &Vec<Positioned<Name>> {
        &self.implements
    }
}
