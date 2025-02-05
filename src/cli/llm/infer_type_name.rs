use std::collections::HashMap;

use genai::chat::{ChatMessage, ChatRequest, ChatResponse};
use indexmap::{indexset, IndexSet};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Error, Result, Wizard};
use crate::core::config::Config;
use crate::core::generator::PREFIX;
use crate::core::Mustache;

const BASE_TEMPLATE: &str = include_str!("prompts/infer_type_name.md");

pub struct InferTypeName {
    wizard: Wizard<Question, Answer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Answer {
    suggestions: Vec<String>,
}

impl TryFrom<ChatResponse> for Answer {
    type Error = Error;

    fn try_from(response: ChatResponse) -> Result<Self> {
        let message_content = response.content.ok_or(Error::EmptyResponse)?;
        let text_content = message_content.text_as_str().ok_or(Error::EmptyResponse)?;
        Ok(serde_json::from_str(text_content)?)
    }
}

#[derive(Clone, Serialize)]
struct Question {
    #[serde(skip_serializing_if = "IndexSet::is_empty")]
    ignore: IndexSet<String>,
    fields: Vec<(String, String)>,
}

#[derive(Serialize)]
struct Context {
    input: Question,
    output: Answer,
}

impl TryInto<ChatRequest> for Question {
    type Error = Error;

    fn try_into(self) -> Result<ChatRequest> {
        let input = Question {
            ignore: indexset! { "User".into()},
            fields: vec![
                ("id".to_string(), "String".to_string()),
                ("name".to_string(), "String".to_string()),
                ("age".to_string(), "Int".to_string()),
            ],
        };

        let output = Answer {
            suggestions: vec![
                "Person".into(),
                "Profile".into(),
                "Member".into(),
                "Individual".into(),
                "Contact".into(),
            ],
        };

        let template = Mustache::parse(BASE_TEMPLATE);

        let context = Context { input, output };

        let rendered_prompt = template.render(&serde_json::to_value(&context)?);

        Ok(ChatRequest::new(vec![
            ChatMessage::system(rendered_prompt),
            ChatMessage::user(serde_json::to_string(&json!({
                "fields": &self.fields,
            }))?),
        ]))
    }
}

impl InferTypeName {
    pub fn new(model: String, secret: Option<String>) -> InferTypeName {
        Self { wizard: Wizard::new(model, secret) }
    }

    /// All generated type names starts with PREFIX
    #[inline]
    fn is_auto_generated(type_name: &str) -> bool {
        type_name.starts_with(PREFIX)
    }

    pub async fn generate(&mut self, config: &Config) -> Result<HashMap<String, String>> {
        let mut new_name_mappings: HashMap<String, String> = HashMap::new();
        // Filter out root operation types and types with non-auto-generated names
        let types_to_be_processed = config
            .types
            .iter()
            .filter(|(type_name, _)| {
                !config.is_root_operation_type(type_name) && Self::is_auto_generated(type_name)
            })
            .collect::<Vec<_>>();

        let mut used_type_names = config
            .types
            .iter()
            .filter(|(ty_name, _)| !Self::is_auto_generated(ty_name))
            .map(|(ty_name, _)| ty_name.to_owned())
            .collect::<IndexSet<_>>();

        let total = types_to_be_processed.len();
        for (i, (type_name, type_)) in types_to_be_processed.into_iter().enumerate() {
            // convert type to sdl format.
            let question = Question {
                ignore: used_type_names.clone(),
                fields: type_
                    .fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.type_of.name().to_owned()))
                    .collect(),
            };

            // Directly use the wizard's ask method to get a result
            let answer = self.wizard.ask(question.clone()).await;

            match answer {
                Ok(answer) => {
                    let name = &answer.suggestions.join(", ");
                    for name in answer.suggestions {
                        if config.types.contains_key(&name) || used_type_names.contains(&name) {
                            continue;
                        }
                        used_type_names.insert(name.clone());
                        new_name_mappings.insert(type_name.to_owned(), name);
                        break;
                    }
                    tracing::info!(
                        "Suggestions for {}: [{}] - {}/{}",
                        type_name,
                        name,
                        i + 1,
                        total
                    );

                    // TODO: case where suggested names are already used, then
                    // extend the base question with
                    // `suggest different names, we have already used following
                    // names: [names list]`
                }
                Err(e) => {
                    // Handle errors in case of failure
                    tracing::error!(
                        "Failed to get suggestions for type '{}': {:?}",
                        type_name,
                        e
                    );
                }
            }
        }

        Ok(new_name_mappings)
    }
}

#[cfg(test)]
mod test {
    use genai::chat::{ChatRequest, ChatResponse, MessageContent};
    use indexmap::indexset;

    use super::{Answer, Question};
    use crate::cli::llm::InferTypeName;

    #[test]
    fn test_to_chat_request_conversion() {
        let question = Question {
            ignore: indexset! {"Profile".to_owned(), "Person".to_owned()},
            fields: vec![
                ("id".to_string(), "String".to_string()),
                ("name".to_string(), "String".to_string()),
                ("age".to_string(), "Int".to_string()),
            ],
        };
        let request: ChatRequest = question.try_into().unwrap();
        insta::assert_debug_snapshot!(request);
    }

    #[test]
    fn test_chat_response_parse() {
        let resp = ChatResponse {
            content: Some(MessageContent::Text(
                "{\"suggestions\":[\"Post\",\"Story\",\"Article\",\"Event\",\"Brief\"]}".to_owned(),
            )),
            ..Default::default()
        };
        let answer = Answer::try_from(resp).unwrap();
        insta::assert_debug_snapshot!(answer);
    }

    #[test]
    fn test_is_auto_generated() {
        assert!(InferTypeName::is_auto_generated("GEN__T1"));
        assert!(InferTypeName::is_auto_generated("GEN__T1234"));
        assert!(InferTypeName::is_auto_generated("GEN__M1"));
        assert!(InferTypeName::is_auto_generated("GEN__M5678"));
        assert!(InferTypeName::is_auto_generated("GEN__Some__Type"));

        assert!(!InferTypeName::is_auto_generated("Some__Type"));
        assert!(!InferTypeName::is_auto_generated("User"));
        assert!(!InferTypeName::is_auto_generated("T123"));
        assert!(!InferTypeName::is_auto_generated("M1"));
        assert!(!InferTypeName::is_auto_generated(""));
        assert!(!InferTypeName::is_auto_generated("123T"));
        assert!(!InferTypeName::is_auto_generated("A1234"));
    }
}
