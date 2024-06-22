use std::collections::{BTreeSet, HashMap};

use convert_case::{Case, Casing};
use prost_reflect::{EnumDescriptor, FieldDescriptor, Kind, MessageDescriptor};

use crate::core::{config::position::Pos, valid::{Valid, Validator}};

// This is an intermediate representation that can help to compare JsonSchemas 
// ensuring that we can identify the position of where the validation error occurred in the source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonScheamWithSourcePosition {
    Obj(HashMap<String, PositionedJsonSchema>),
    Arr(Box<PositionedJsonSchema>),
    Opt(Box<PositionedJsonSchema>),
    Enum(BTreeSet<String>),
    Str,
    Num,
    Bool,
    Empty,
    Any,
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct SourcePos(pub usize, pub usize, pub Option<String>);

impl<T> From<&Pos<T>> for SourcePos {
    fn from(value: &Pos<T>) -> Self {
        Self(value.line, value.column, value.file_path.clone())
    }
}

impl SourcePos {
    pub fn to_pos_trace_err(&self) -> Option<String> {
        Some(format!("{} {}#{}", self.2.as_ref().unwrap(), self.0, self.1))
    }

    pub fn is_pos_trace_supported(&self) -> bool {
        self.2.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionedJsonSchema {
    pub schema: JsonScheamWithSourcePosition,
    pub source_position: SourcePos,
}

impl std::fmt::Display for PositionedJsonSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.schema)
    }
}

impl std::fmt::Display for JsonScheamWithSourcePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonScheamWithSourcePosition::Arr(inner) => write!(f, "Arr({})", inner),
            JsonScheamWithSourcePosition::Opt(inner) => write!(f, "Opt({})", inner),
            JsonScheamWithSourcePosition::Enum(inner) => write!(f, "Enum({:?})", inner),
            JsonScheamWithSourcePosition::Obj(_) => write!(f, "Obj"),
            JsonScheamWithSourcePosition::Str => write!(f, "Str"),
            JsonScheamWithSourcePosition::Num => write!(f, "Num"),
            JsonScheamWithSourcePosition::Bool => write!(f, "Bool"),
            JsonScheamWithSourcePosition::Empty => write!(f, "Empty"),
            JsonScheamWithSourcePosition::Any => write!(f, "Any"),
        }
    }
}

impl PositionedJsonSchema {
    pub fn new(schema: JsonScheamWithSourcePosition, source_position: SourcePos) -> Self {
        Self { schema, source_position }
    }

    pub fn compare(&self, other: &PositionedJsonSchema, name: &str) -> Valid<(), String> {
        let mut trace_err = Some(name);
        let positioned_err = self.source_position.to_pos_trace_err();
        let mut field_name = None;

        if self.source_position.is_pos_trace_supported() {
            trace_err = positioned_err.as_deref();
        } else {
            field_name = Some(name);
        }

        match &self.schema {
            JsonScheamWithSourcePosition::Str => {
                if other.schema != self.schema {
                    return Valid::fail(format!("expected String, got {}", other.schema))
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Num => {
                if other.schema != self.schema {
                    return Valid::fail(format!("expected Number, got {}", other.schema))
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Bool => {
                if other.schema != self.schema {
                    return Valid::fail(format!("expected Boolean, got {}", other.schema))
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Empty => {
                if other.schema != self.schema {
                    return Valid::fail(format!("expected Empty, got {}", other.schema))
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Any => {
                if other.schema != self.schema {
                    return Valid::fail(format!("expected Any, got {}", other.schema))
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Obj(a) => {
                if let JsonScheamWithSourcePosition::Obj(b) = &other.schema {
                    return Valid::from_iter(b.iter(), |(key, b)| {
                        Valid::from_option(a.get(key), format!("missing key: {}", key))
                            .trace(trace_err)
                            .and_then(|a| a.compare(b, key))
                    })
                    .trace(field_name)
                    .unit();
                } else {
                    return Valid::fail("expected Object type".to_string())
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Arr(a) => {
                if let JsonScheamWithSourcePosition::Arr(b) = &other.schema {
                    return a.compare(b, name);
                } else {
                    return Valid::fail("expected Non repeatable type".to_string())
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Opt(a) => {
                if let JsonScheamWithSourcePosition::Opt(b) = &other.schema {
                    return a.compare(b, name);
                } else {
                    return Valid::fail("expected type to be required".to_string())
                        .trace(trace_err);
                }
            }
            JsonScheamWithSourcePosition::Enum(a) => {
                if let JsonScheamWithSourcePosition::Enum(b) = &other.schema {
                    if a.ne(b) {
                        return Valid::fail(format!("expected {:?} but found {:?}", a, b))
                            .trace(trace_err);
                    }
                } else {
                    return Valid::fail(format!("expected Enum got: {:?}", other.schema))
                        .trace(trace_err);
                }
            }
        }
        Valid::succeed(())
    }
}

impl TryFrom<&MessageDescriptor> for PositionedJsonSchema {
    type Error = crate::core::valid::ValidationError<String>;

    fn try_from(value: &MessageDescriptor) -> Result<Self, Self::Error> {
        Ok(PositionedJsonSchema {
            schema: JsonScheamWithSourcePosition::try_from(value)?,
            source_position: Default::default(),
        })
    }
}

impl TryFrom<&MessageDescriptor> for JsonScheamWithSourcePosition {
    type Error = crate::core::valid::ValidationError<String>;

    fn try_from(value: &MessageDescriptor) -> Result<Self, Self::Error> {
        if value.is_map_entry() {
            // we encode protobuf's map as JSON scalar
            return Ok(JsonScheamWithSourcePosition::Any);
        }

        let mut map = std::collections::HashMap::new();
        let fields = value.fields();

        for field in fields {
            let field_schema = PositionedJsonSchema::try_from(&field)?;

            // the snake_case for field names is automatically converted to camelCase
            // by prost on serde serialize/deserealize and in graphql type name should be in
            // camelCase as well, so convert field.name to camelCase here
            map.insert(field.name().to_case(Case::Camel), field_schema);
        }

        if map.is_empty() {
            Ok(JsonScheamWithSourcePosition::Empty)
        } else {
            Ok(JsonScheamWithSourcePosition::Obj(map))
        }
    }
}

impl TryFrom<&EnumDescriptor> for JsonScheamWithSourcePosition {
    type Error = crate::core::valid::ValidationError<String>;

    fn try_from(value: &EnumDescriptor) -> Result<Self, Self::Error> {
        let mut set = BTreeSet::new();
        for value in value.values() {
            set.insert(value.name().to_string());
        }
        Ok(JsonScheamWithSourcePosition::Enum(set))
    }
}

impl TryFrom<&FieldDescriptor> for PositionedJsonSchema {
    type Error = crate::core::valid::ValidationError<String>;

    fn try_from(value: &FieldDescriptor) -> Result<Self, Self::Error> {
        let field_schema = match value.kind() {
            Kind::Double => JsonScheamWithSourcePosition::Num,
            Kind::Float => JsonScheamWithSourcePosition::Num,
            Kind::Int32 => JsonScheamWithSourcePosition::Num,
            Kind::Int64 => JsonScheamWithSourcePosition::Num,
            Kind::Uint32 => JsonScheamWithSourcePosition::Num,
            Kind::Uint64 => JsonScheamWithSourcePosition::Num,
            Kind::Sint32 => JsonScheamWithSourcePosition::Num,
            Kind::Sint64 => JsonScheamWithSourcePosition::Num,
            Kind::Fixed32 => JsonScheamWithSourcePosition::Num,
            Kind::Fixed64 => JsonScheamWithSourcePosition::Num,
            Kind::Sfixed32 => JsonScheamWithSourcePosition::Num,
            Kind::Sfixed64 => JsonScheamWithSourcePosition::Num,
            Kind::Bool => JsonScheamWithSourcePosition::Bool,
            Kind::String => JsonScheamWithSourcePosition::Str,
            Kind::Bytes => JsonScheamWithSourcePosition::Str,
            Kind::Message(msg) => JsonScheamWithSourcePosition::try_from(&msg)?,
            Kind::Enum(enm) => JsonScheamWithSourcePosition::try_from(&enm)?,
        };
        let field_schema = if value
            .cardinality()
            .eq(&prost_reflect::Cardinality::Optional)
        {
            JsonScheamWithSourcePosition::Opt(Box::new(Self {
                schema: field_schema,
                source_position: Default::default(),
            }))
        } else {
            field_schema
        };
        let field_schema = if value.is_list() {
            Self {
                schema: JsonScheamWithSourcePosition::Arr(Box::new(Self {
                    schema: field_schema,
                    source_position: Default::default(),
                })),
                source_position: Default::default(),
            }
        } else {
            Self { schema: field_schema, source_position: Default::default() }
        };

        Ok(field_schema)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use tailcall_fixtures::protobuf;

    use crate::core::blueprint::GrpcMethod;
    use crate::core::grpc::protobuf::tests::get_proto_file;
    use crate::core::grpc::protobuf::ProtobufSet;
    use crate::core::json::{JsonScheamWithSourcePosition, PositionedJsonSchema};
    use crate::core::valid::{Valid, Validator};

    #[test]
    fn test_compare_enum() {
        let mut en = BTreeSet::new();
        en.insert("A".to_string());
        en.insert("B".to_string());
        let value = PositionedJsonSchema::new(
            JsonScheamWithSourcePosition::Arr(Box::new(PositionedJsonSchema::new(
                JsonScheamWithSourcePosition::Enum(en.clone()),
                Default::default(),
            ))),
            Default::default(),
        );
        let schema =
            PositionedJsonSchema::new(JsonScheamWithSourcePosition::Enum(en), Default::default());
        let name = "foo";
        let result = schema.compare(&value, name);
        assert_eq!(
            result,
            Valid::fail("expected Enum got: Arr(Enum({\"A\", \"B\"}))".to_string())
                .trace(Some(name))
        );
    }

    #[test]
    fn test_compare_enum_value() {
        let mut en = BTreeSet::new();
        en.insert("A".to_string());
        en.insert("B".to_string());

        let mut en1 = BTreeSet::new();
        en1.insert("A".to_string());
        en1.insert("B".to_string());
        en1.insert("C".to_string());

        let value = PositionedJsonSchema::new(
            JsonScheamWithSourcePosition::Enum(en1.clone()),
            Default::default(),
        );
        let schema = PositionedJsonSchema::new(
            JsonScheamWithSourcePosition::Enum(en.clone()),
            Default::default(),
        );
        let name = "foo";
        let result = schema.compare(&value, name);
        assert_eq!(
            result,
            Valid::fail("expected {\"A\", \"B\"} but found {\"A\", \"B\", \"C\"}".to_string())
                .trace(Some(name))
        );
    }

    #[tokio::test]
    async fn test_from_protobuf_conversion() -> anyhow::Result<()> {
        let grpc_method = GrpcMethod::try_from("news.NewsService.GetNews").unwrap();

        let file = ProtobufSet::from_proto_file(get_proto_file(protobuf::NEWS).await?)?;
        let service = file.find_service(&grpc_method)?;
        let operation = service.find_operation(&grpc_method)?;

        let schema = PositionedJsonSchema::try_from(&operation.output_type)?;

        let expected = PositionedJsonSchema::new(
            JsonScheamWithSourcePosition::Obj(HashMap::from_iter([
                (
                    "postImage".to_owned(),
                    PositionedJsonSchema::new(
                        JsonScheamWithSourcePosition::Opt(
                            PositionedJsonSchema::new(
                                JsonScheamWithSourcePosition::Str,
                                Default::default(),
                            )
                            .into(),
                        ),
                        Default::default(),
                    ),
                ),
                (
                    "title".to_owned(),
                    PositionedJsonSchema::new(
                        JsonScheamWithSourcePosition::Opt(
                            PositionedJsonSchema::new(
                                JsonScheamWithSourcePosition::Str,
                                Default::default(),
                            )
                            .into(),
                        ),
                        Default::default(),
                    ),
                ),
                (
                    "id".to_owned(),
                    PositionedJsonSchema::new(
                        JsonScheamWithSourcePosition::Opt(
                            PositionedJsonSchema::new(
                                JsonScheamWithSourcePosition::Num,
                                Default::default(),
                            )
                            .into(),
                        ),
                        Default::default(),
                    ),
                ),
                (
                    "body".to_owned(),
                    PositionedJsonSchema::new(
                        JsonScheamWithSourcePosition::Opt(
                            PositionedJsonSchema::new(
                                JsonScheamWithSourcePosition::Str,
                                Default::default(),
                            )
                            .into(),
                        ),
                        Default::default(),
                    ),
                ),
                (
                    "status".to_owned(),
                    PositionedJsonSchema::new(
                        JsonScheamWithSourcePosition::Opt(
                            PositionedJsonSchema::new(
                                JsonScheamWithSourcePosition::Enum(BTreeSet::from_iter([
                                    "DELETED".to_owned(),
                                    "DRAFT".to_owned(),
                                    "PUBLISHED".to_owned(),
                                ])),
                                Default::default(),
                            )
                            .into(),
                        ),
                        Default::default(),
                    ),
                ),
            ])),
            Default::default(),
        );

        assert_eq!(schema, expected);

        Ok(())
    }
}
