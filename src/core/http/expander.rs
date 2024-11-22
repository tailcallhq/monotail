use serde_json::Value;

pub struct Expand;

impl Expand {
    // Takes ownership of the request body and returns the expanded Value.
    pub fn expand(value: Value, batch_size: usize) -> Value {
        match value {
            Value::Object(map) => {
                let expanded_map = map
                    .into_iter()
                    .map(|(k, v)| (k, Self::expand(v, batch_size)))
                    .collect();
                Value::Object(expanded_map)
            }
            Value::Array(list) => {
                let expanded_list: Vec<Value> = list
                    .into_iter()
                    .map(|v| Self::expand(v, batch_size))
                    .collect();

                let mut final_ans = Vec::with_capacity(expanded_list.len());

                for index in 0..batch_size {
                    let expanded_batch: Vec<Value> = expanded_list
                        .iter()
                        .cloned()
                        .map(|v| Self::update_mustache_expr(v, index))
                        .collect();
                    final_ans.extend(expanded_batch);
                }
                Value::Array(final_ans)
            }
            other => other, // Return as is for other variants.
        }
    }

    fn update_mustache_expr(value: Value, index: usize) -> Value {
        match value {
            Value::Object(map) => {
                let updated_map = map
                    .into_iter()
                    .map(|(k, v)| (k, Self::update_mustache_expr(v, index)))
                    .collect();
                Value::Object(updated_map)
            }
            Value::Array(list) => {
                let updated_list = list
                    .into_iter()
                    .map(|v| Self::update_mustache_expr(v, index))
                    .collect();
                Value::Array(updated_list)
            }
            Value::String(s) => {
                if s.contains("{{.value.") || s.contains("{{value.") {
                    let updated_string = s
                        .replace("{{.value.", &format!("{{{{.value.{}.", index))
                        .replace("{{value.", &format!("{{{{value.{}.", index));
                    Value::String(updated_string)
                } else {
                    Value::String(s)
                }
            }
            other => other, // Return as is for other variants.
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::Mustache;

    use super::*;
    use serde_json::json;

    #[test]
    fn test_expander() {
        // Test Option 1
        let input1 = json!({
            "a": { "b": { "c": { "d": ["{{.value.userId}}"] } } }
        });

        let expanded1 = Expand::expand(input1, 2);
        println!("expanded: {:#?}", Mustache::parse(&expanded1.to_string()));

        let input2 = json!([{ "userId": "{{.value.id}}", "title": "{{.value.name}}","content": "Hello World" }]);
        let expanded2 = Expand::expand(input2, 2);
        println!("expanded: {:#?}", Mustache::parse(&expanded2.to_string()));

        // Option 3:
        let input3 = json!([{ "metadata": "xyz", "items": "{{.value.userId}}" }]);
        let expanded3 = Expand::expand(input3, 2);
        println!("expanded: {:#?}", Mustache::parse(&expanded3.to_string()));

        // Option 4:
        let input4 =
            json!({ "metadata": "xyz", "items": [{"key": "id", "value": "{{.value.userId}}" }]} );
        let expanded4 = Expand::expand(input4, 2);
        println!("expanded: {:#?}", Mustache::parse(&expanded4.to_string()));
    }
}
