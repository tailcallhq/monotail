use std::collections::HashSet;

use indexmap::IndexMap;

use super::InputObjectTypeDefinition;
use crate::core::blueprint::{
    Blueprint, Definition, FieldDefinition, InputFieldDefinition, SchemaDefinition,
};
use crate::core::scalar;

///
/// A read optimized index of all the types in the Blueprint. Provide O(1)
/// access to getting any field information.

#[derive(Debug)]
pub struct Index {
    map: IndexMap<String, (Definition, IndexMap<String, QueryField>)>,
    schema: SchemaDefinition,
}

#[derive(Debug)]
pub enum QueryField {
    Field((FieldDefinition, IndexMap<String, InputFieldDefinition>)),
    InputField(InputFieldDefinition),
}

impl QueryField {
    pub fn get_arg(&self, arg_name: &str) -> Option<&InputFieldDefinition> {
        match self {
            QueryField::Field((_, args)) => args.get(arg_name),
            QueryField::InputField(_) => None,
        }
    }
}

impl Index {
    pub fn type_is_scalar(&self, type_name: &str) -> bool {
        let def = self.map.get(type_name).map(|(def, _)| def);

        matches!(def, Some(Definition::Scalar(_))) || scalar::Scalar::is_predefined(type_name)
    }

    pub fn get_interfaces(&self) -> HashSet<String> {
        self.map
            .iter()
            .filter_map(|(_, (def, _))| match def {
                Definition::Interface(interface) => Some(interface.name.to_owned()),
                _ => None,
            })
            .collect()
    }

    pub fn type_is_enum(&self, type_name: &str) -> bool {
        let def = self.map.get(type_name).map(|(def, _)| def);

        matches!(def, Some(Definition::Enum(_)))
    }

    pub fn validate_enum_value(&self, type_name: &str, value: &str) -> bool {
        let def = self.map.get(type_name).map(|(def, _)| def);

        if let Some(Definition::Enum(enum_)) = def {
            enum_
                .enum_values
                .iter()
                .any(|v| v.name == value || v.alias.contains(value))
        } else {
            false
        }
    }

    pub fn get_field(&self, type_name: &str, field_name: &str) -> Option<&QueryField> {
        self.map
            .get(type_name)
            .and_then(|(_, fields_map)| fields_map.get(field_name))
    }

    pub fn get_query(&self) -> &String {
        &self.schema.query
    }

    pub fn get_mutation(&self) -> Option<&str> {
        self.schema.mutation.as_deref()
    }

    pub fn is_type_implements(&self, type_name: &str, type_or_interface: &str) -> bool {
        if type_name == type_or_interface {
            return true;
        }

        if let Some((Definition::Object(obj), _)) = self.map.get(type_name) {
            obj.implements.contains(type_or_interface)
        } else {
            false
        }
    }

    pub fn get_input_type_definition(&self, type_name: &str) -> Option<&InputObjectTypeDefinition> {
        match self.map.get(type_name) {
            Some((Definition::InputObject(input), _)) => Some(input),
            _ => None,
        }
    }
}

impl From<&Blueprint> for Index {
    fn from(blueprint: &Blueprint) -> Self {
        let mut map = IndexMap::new();

        for definition in blueprint.definitions.iter() {
            match definition {
                Definition::Object(object_def) => {
                    let type_name = object_def.name.clone();
                    let mut fields_map = IndexMap::new();

                    for field in &object_def.fields {
                        let args_map = IndexMap::from_iter(
                            field
                                .args
                                .iter()
                                .map(|v| (v.name.clone(), v.clone()))
                                .collect::<Vec<_>>(),
                        );
                        fields_map.insert(
                            field.name.clone(),
                            QueryField::Field((field.clone(), args_map)),
                        );
                    }

                    map.insert(
                        type_name,
                        (Definition::Object(object_def.to_owned()), fields_map),
                    );
                }
                Definition::Interface(interface_def) => {
                    let type_name = interface_def.name.clone();
                    let mut fields_map = IndexMap::new();

                    for field in interface_def.fields.clone() {
                        let args_map = IndexMap::from_iter(
                            field
                                .args
                                .iter()
                                .map(|v| (v.name.clone(), v.clone()))
                                .collect::<Vec<_>>(),
                        );
                        fields_map.insert(field.name.clone(), QueryField::Field((field, args_map)));
                    }

                    map.insert(
                        type_name,
                        (Definition::Interface(interface_def.to_owned()), fields_map),
                    );
                }
                Definition::InputObject(input_object_def) => {
                    let type_name = input_object_def.name.clone();
                    let mut fields_map = IndexMap::new();

                    for field in input_object_def.fields.clone() {
                        fields_map.insert(field.name.clone(), QueryField::InputField(field));
                    }

                    map.insert(
                        type_name,
                        (
                            Definition::InputObject(input_object_def.to_owned()),
                            fields_map,
                        ),
                    );
                }
                Definition::Scalar(scalar_def) => {
                    let type_name = scalar_def.name.clone();
                    map.insert(
                        type_name.clone(),
                        (Definition::Scalar(scalar_def.to_owned()), IndexMap::new()),
                    );
                }
                Definition::Enum(enum_def) => {
                    let type_name = enum_def.name.clone();
                    map.insert(
                        type_name.clone(),
                        (Definition::Enum(enum_def.to_owned()), IndexMap::new()),
                    );
                }
                Definition::Union(union_def) => {
                    let type_name = union_def.name.clone();
                    map.insert(
                        type_name.clone(),
                        (Definition::Union(union_def.to_owned()), IndexMap::new()),
                    );
                }
            }
        }

        Self { map, schema: blueprint.schema.to_owned() }
    }
}

#[cfg(test)]
mod test {
    use super::Index;
    use crate::core::blueprint::Blueprint;
    use crate::core::config::ConfigModule;
    use crate::include_config;

    fn setup() -> Index {
        let config = include_config!("./fixture/all-constructs.graphql").unwrap();
        let cfg_module = ConfigModule::from(config);
        let blueprint = Blueprint::try_from(&cfg_module).unwrap();

        Index::from(&blueprint)
    }

    #[test]
    fn test_from_blueprint() {
        let index = setup();
        insta::assert_debug_snapshot!(index);
    }

    #[test]
    fn test_is_scalar() {
        let index = setup();
        assert!(index.type_is_scalar("Int"));
        assert!(index.type_is_scalar("String"));

        assert!(!index.type_is_scalar("Color"));
    }

    #[test]
    fn test_is_enum() {
        let index = setup();
        assert!(index.type_is_enum("Status"));
        assert!(!index.type_is_enum("Int"));
    }

    #[test]
    fn test_validate_enum_value() {
        let index = setup();
        assert!(index.validate_enum_value("Status", "ACTIVE"));
        assert!(!index.validate_enum_value("Status", "YELLOW"));
        assert!(!index.validate_enum_value("Int", "1"));
    }

    #[test]
    fn test_get_field() {
        let index = setup();
        assert!(index.get_field("Query", "user").is_some());
        assert!(index.get_field("Query", "non_existent_field").is_none());
        assert!(index.get_field("Status", "Pending").is_none());
    }

    #[test]
    fn test_get_query() {
        let index = setup();
        assert_eq!(index.get_query(), "Query");
    }

    #[test]
    fn test_get_mutation() {
        let index = setup();
        assert_eq!(index.get_mutation(), Some("Mutation"));
    }

    #[test]
    fn test_get_mutation_none() {
        let mut index = setup();
        index.schema.mutation = None;
        assert_eq!(index.get_mutation(), None);
    }

    #[test]
    fn test_is_type_implements() {
        let index = setup();

        assert!(index.is_type_implements("User", "Node"));
        assert!(index.is_type_implements("Post", "Post"));
        assert!(!index.is_type_implements("Node", "User"));
    }
}
