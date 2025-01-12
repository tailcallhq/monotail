use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use tailcall_valid::Valid;

use crate::core::document::print_directives;
use crate::core::ir::model::{IO, IR};
use crate::core::jit::{Field, OperationPlan};
use crate::core::json::JsonLikeOwned;
use crate::core::{Mustache, Transform};

#[derive(Default)]
pub struct GraphQL<A>(PhantomData<A>);

impl<A> GraphQL<A> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

fn compute_selection_set<A: Display + Debug + JsonLikeOwned>(
    base_field: &mut [Field<A>],
    interfaces: &HashSet<String>,
) {
    for field in base_field.iter_mut() {
        if let Some(ir) = field.ir.as_mut() {
            ir.modify_io(&mut |io| {
                if let IO::GraphQL { req_template, .. } = io {
                    if let Some(v) = format_selection_set(
                        field.selection.iter(),
                        interfaces,
                        interfaces.contains(field.type_of.name()),
                    ) {
                        req_template.selection = Some(Mustache::parse(&v).into());
                    }
                }
            });
        }
        compute_selection_set(field.selection.as_mut(), interfaces);
    }
}

impl<A: Display + Debug + JsonLikeOwned + Clone> Transform for GraphQL<A> {
    type Value = OperationPlan<A>;
    type Error = Infallible;

    fn transform(&self, mut plan: Self::Value) -> Valid<Self::Value, Self::Error> {
        let interfaces = match plan.interfaces {
            Some(ref interfaces) => interfaces,
            None => &HashSet::new(),
        };
        compute_selection_set(&mut plan.selection, interfaces);

        Valid::succeed(plan)
    }
}

fn format_selection_set<'a, A: 'a + Display + JsonLikeOwned>(
    selection_set: impl Iterator<Item = &'a Field<A>>,
    interfaces: &HashSet<String>,
    is_parent_interface: bool,
) -> Option<String> {
    let mut fragments_fields = HashMap::new();
    let mut normal_fields = vec![];
    let mut is_typename_requested = false;
    let set = selection_set
        .filter(|field| !matches!(&field.ir, Some(IR::IO(_)) | Some(IR::Dynamic(_))))
        .map(|field| {
            // handle @modify directive scenario.
            let field_name = if let Some(IR::ContextPath(data)) = &field.ir {
                data.first().cloned().unwrap_or(field.name.to_string())
            } else {
                field.name.to_string()
            };
            let is_this_field_interface = interfaces.contains(field.type_of.name());
            let formatted_selection_fields =
                format_selection_field(field, &field_name, interfaces, is_this_field_interface);
            is_typename_requested = is_typename_requested || field_name == "__typename";
            match &field.parent_fragment {
                Some(fragment) if is_parent_interface => {
                    fragments_fields
                        .entry(fragment.to_owned())
                        .or_insert_with(Vec::new)
                        .push(formatted_selection_fields);
                }
                _ => {
                    normal_fields.push(formatted_selection_fields);
                }
            }
        })
        .collect::<Vec<_>>();

    if set.is_empty() {
        return None;
    }

    let fragments_set: Vec<String> = fragments_fields
        .into_iter()
        .map(|(fragment_name, fields)| {
            format!("... on {} {{ {} }}", fragment_name, fields.join(" "))
        })
        .collect();

    //Don't force user to query the type and get it automatically
    if is_parent_interface && !is_typename_requested {
        normal_fields.push("__typename".to_owned());
    }
    normal_fields.extend(fragments_set);
    Some(format!("{{ {} }}", normal_fields.join(" ")))
}

fn format_selection_field<A: Display + JsonLikeOwned>(
    field: &Field<A>,
    name: &str,
    interfaces: &HashSet<String>,
    is_parent_interface: bool,
) -> String {
    let arguments = format_selection_field_arguments(field);
    let selection_set =
        format_selection_set(field.selection.iter(), interfaces, is_parent_interface);

    let mut output = format!("{}{}", name, arguments);

    if !field.directives.is_empty() {
        let directives = print_directives(field.directives.iter());

        if !directives.is_empty() {
            output.push(' ');
            output.push_str(&directives.escape_default().to_string());
        }
    }

    if let Some(selection_set) = selection_set {
        output.push(' ');
        output.push_str(&selection_set);
    }

    output
}

fn format_selection_field_arguments<A: Display>(field: &Field<A>) -> Cow<'static, str> {
    let arguments = field
        .args
        .iter()
        .filter(|a| a.value.is_some())
        .map(|arg| arg.to_string())
        .collect::<Vec<_>>()
        .join(",");

    if arguments.is_empty() {
        Cow::Borrowed("")
    } else {
        Cow::Owned(format!("({})", arguments.escape_default()))
    }
}
