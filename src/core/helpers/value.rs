use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use async_graphql_value::ConstValue;

#[derive(Clone, Eq)]
pub struct HashableConstValue(pub ConstValue);

impl Hash for HashableConstValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash(&self.0, state)
    }
}

impl PartialEq for HashableConstValue {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

pub fn hash<H: Hasher>(const_value: &ConstValue, state: &mut H) {
    match const_value {
        ConstValue::Null => {}
        ConstValue::Boolean(val) => val.hash(state),
        ConstValue::Enum(name) => name.hash(state),
        ConstValue::Number(num) => num.hash(state),
        ConstValue::Binary(bytes) => bytes.hash(state),
        ConstValue::String(string) => string.hash(state),
        ConstValue::List(list) => list.iter().for_each(|val| hash(val, state)),
        ConstValue::Object(object) => {
            let mut tmp_list: Vec<_> = object.iter().collect();
            tmp_list.sort_by(|(key1, _), (key2, _)| key1.cmp(key2));
            tmp_list.iter().for_each(|(key, value)| {
                key.hash(state);
                hash(value, state);
            })
        }
    }
}

pub fn arc_result_to_result<T: Clone, E: Clone>(arc_result: Arc<Result<T, E>>) -> Result<T, E> {
    match &*arc_result {
        Ok(t) => Ok(t.clone()),
        Err(e) => Err(e.clone()),
    }
}
