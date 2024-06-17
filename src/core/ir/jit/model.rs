use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};

use tailcall_hasher::TailcallHasher;

use super::Result;
use crate::core::ir::model::IR;

const EMPTY_VEC: &Vec<Field<Children>> = &Vec::new();
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    // A representation where nodes are connect to their parent (one to one)
    pub parent: Vec<Field<Parent>>,

    // A representation where nodes are connect to their children (one to many)
    pub children: Vec<Field<Children>>,

    // A pre-resolved value of the execution plan
    #[allow(unused)]
    pub value: Option<Result<Vec<u8>>>,

    // Inferred from the IR, if the execution can be deduplicated
    #[allow(unused)]
    pub dedupe: bool,

    // Inferred from the IR, if the execution requires Authentication
    #[allow(unused)]
    pub authenticate: bool,

    // Inferred from the IR, if the execution has IO
    #[allow(unused)]
    pub has_io: bool,

    // A unique Identifier for the execution plan
    #[allow(unused)]
    pub id: u64,
}

impl ExecutionPlan {
    pub fn new(parent: Vec<Field<Parent>>) -> Self {
        let children = parent
            .iter()
            .filter(|f| f.refs.is_none())
            .map(|f| f.to_owned().into_children(&parent))
            .collect::<Vec<_>>();

        let mut hasher = TailcallHasher::default();
        children.hash(&mut hasher);
        let id = hasher.finish();

        Self {
            id,
            parent,
            children,
            dedupe: false,
            authenticate: false,
            value: None,
            has_io: false,
        }
    }

    #[allow(unused)]
    pub fn as_children(&self) -> &[Field<Children>] {
        &self.children
    }

    #[allow(unused)]
    pub fn as_parent(&self) -> &[Field<Parent>] {
        &self.parent
    }

    #[allow(unused)]
    pub fn find_field(&self, id: FieldId) -> Option<&Field<Parent>> {
        self.parent.iter().find(|field| field.id == id)
    }
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct Arg {
    pub id: ArgId,
    pub name: String,
    pub type_of: crate::core::blueprint::Type,
    pub value: Option<async_graphql_value::Value>,
    pub default_value: Option<async_graphql_value::ConstValue>,
}

impl Hash for Arg {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

#[derive(Clone)]
pub struct ArgId(usize);

impl Debug for ArgId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ArgId {
    pub fn new(id: usize) -> Self {
        ArgId(id)
    }
}

#[derive(Clone)]
pub struct Field<A: Clone> {
    pub id: FieldId,
    pub name: String,
    pub ir: Option<IR>,
    pub type_of: crate::core::blueprint::Type,
    pub args: Vec<Arg>,
    pub refs: Option<A>,
}

impl<A: Hash + Clone> Hash for Field<A> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.args.hash(state);
        self.refs.hash(state);
    }
}

impl Field<Children> {
    #[allow(unused)]
    pub fn children(&self) -> &Vec<Field<Children>> {
        match &self.refs {
            Some(Children(children)) => children,
            _ => EMPTY_VEC,
        }
    }
}

impl Field<Parent> {
    fn parent(&self) -> Option<&FieldId> {
        self.refs.as_ref().map(|Parent(id)| id)
    }

    fn into_children(self, fields: &[Field<Parent>]) -> Field<Children> {
        let mut children = Vec::new();
        for field in fields.iter() {
            if let Some(id) = field.parent() {
                if *id == self.id {
                    children.push(field.to_owned().into_children(fields));
                }
            }
        }

        let refs = if children.is_empty() {
            None
        } else {
            Some(Children(children))
        };

        Field {
            id: self.id,
            name: self.name,
            ir: self.ir,
            type_of: self.type_of,
            args: self.args,
            refs,
        }
    }
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

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FieldId(usize);

impl FieldId {
    pub fn new(id: usize) -> Self {
        FieldId(id)
    }
}

impl Debug for FieldId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone)]
pub struct Parent(FieldId);
#[allow(unused)]
impl Parent {
    pub fn new(id: FieldId) -> Self {
        Parent(id)
    }
}
impl Debug for Parent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Parent({:?})", self.0)
    }
}

#[derive(Clone, Debug, Hash)]
pub struct Children(Vec<Field<Children>>);
