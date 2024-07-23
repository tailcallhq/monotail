use schemars::JsonSchema;
use tailcall_macros::ScalarDefinition;

use crate::core::json::{JsonLike};

/// A field whose value conforms to the standard E.164 format as specified in E.164 specification (https://en.wikipedia.org/wiki/E.164).
#[derive(JsonSchema, Default, ScalarDefinition)]
pub struct PhoneNumber {
    #[allow(dead_code)]
    #[serde(rename = "PhoneNumber")]
    pub phone_no: String,
}
impl super::Scalar for PhoneNumber {
    /// Function used to validate the phone number
    fn validate<'a, Value: JsonLike<'a>>(&self) -> fn(&'a Value) -> bool {
        |value: &'a Value| {
            if let Some(phone_str) = value.as_str() {
                return phonenumber::parse(None, phone_str).is_ok();
            }
            false
        }
    }
}

#[cfg(test)]
mod test {
    use async_graphql_value::ConstValue;

    use super::*;
    use crate::core::scalar::Scalar;

    #[test]
    fn test_phone_number() {
        let phone = PhoneNumber::default();
        let validate = phone.validate()(&ConstValue::String("+911234567890".to_string()));
        assert!(validate);
    }

    #[test]
    fn test_invalid_phone_number() {
        let phone = PhoneNumber::default();
        let validate = phone.validate()(&ConstValue::String("1234567890".to_string()));
        assert!(!validate);
    }

    #[test]
    fn test_invalid_value() {
        let phone = PhoneNumber::default();
        let validate = phone.validate()(&ConstValue::Null);
        assert!(!validate);
    }
}
