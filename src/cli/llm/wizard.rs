use derive_setters::Setters;
use genai::adapter::AdapterKind;
use genai::chat::{ChatOptions, ChatRequest, ChatResponse};
use genai::resolver::AuthResolver;
use genai::Client;
use reqwest::StatusCode;
use tokio_retry::strategy::ExponentialBackoff;
use tokio_retry::RetryIf;

use super::error::{Error, Result};

#[derive(Setters, Clone)]
pub struct Wizard<Q, A> {
    client: Client,
    model: String,
    _q: std::marker::PhantomData<Q>,
    _a: std::marker::PhantomData<A>,
}

impl<Q, A> Wizard<Q, A> {
    pub fn new(model: String, secret: Option<String>) -> Self {
        let mut config = genai::adapter::AdapterConfig::default();
        if let Some(key) = secret {
            config = config.with_auth_resolver(AuthResolver::from_key_value(key));
        }

        let adapter = AdapterKind::from_model(model.as_str()).unwrap_or(AdapterKind::Ollama);

        let chat_options = ChatOptions::default()
            .with_json_mode(true)
            .with_temperature(0.0);

        Self {
            client: Client::builder()
                .with_chat_options(chat_options)
                .insert_adapter_config(adapter, config)
                .build(),
            model,
            _q: Default::default(),
            _a: Default::default(),
        }
    }

    pub async fn ask(&self, q: Q) -> Result<A>
    where
        Q: TryInto<ChatRequest, Error = super::Error> + Clone,
        A: TryFrom<ChatResponse, Error = super::Error>,
    {
        let retry_strategy = ExponentialBackoff::from_millis(500)
            .max_delay(std::time::Duration::from_secs(30))
            .take(5);

        RetryIf::spawn(
            retry_strategy,
            || async {
                let request = q.clone().try_into()?; // Convert the question to a request
                self.client
                    .exec_chat(self.model.as_str(), request, None) // Execute chat request
                    .await
                    .map_err(Error::from)
                    .and_then(A::try_from) // Convert the response into the
                                           // desired result
            },
            |err: &Error| {
                // Check if the error is a ReqwestError and if the status is 429
                if let Error::Reqwest(reqwest_err) = err {
                    if let Some(status) = reqwest_err.status() {
                        return status == StatusCode::TOO_MANY_REQUESTS;
                    }
                }
                false
            },
        )
        .await
    }
}
