#![allow(unused)]

use std::borrow::Cow;
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;

use derive_setters::Setters;
use genai::adapter::AdapterKind;

#[derive(Clone, Setters)]
pub struct Model {
    pub name: &'static str,
    pub secret: Option<String>,
}

pub struct OpenAI;
pub struct Ollama;
pub struct Anthropic;
pub struct Cohere;
pub struct Gemini;
pub struct Groq;

impl Model {
    pub const OPEN_AI: OpenAI = OpenAI;
    pub const OLLAMA: Ollama = Ollama;
    pub const ANTHROPIC: Anthropic = Anthropic;
    pub const COHERE: Cohere = Cohere;
    pub const GEMINI: Gemini = Gemini;
    pub const GROQ: Groq = Groq;

    pub fn config(&self) -> genai::adapter::AdapterConfig {
        let mut config = genai::adapter::AdapterConfig::default();
        if let Some(key) = self.secret.clone() {
            config = config.with_auth_env_name(key);
        }
        config
    }

    pub fn to_adapter_kind(&self) -> genai::adapter::AdapterKind {
        // should be safe to call unwrap here
        AdapterKind::from_model(self.name).unwrap()
    }
}

impl OpenAI {
    pub fn gpt3_5_turbo(&self) -> Model {
        Model { name: "gp-3.5-turbo", secret: None }
    }
    pub fn gpt4(&self) -> Model {
        Model { name: "gpt-4", secret: None }
    }
    pub fn gpt4_turbo(&self) -> Model {
        Model { name: "gpt-4-turbo", secret: None }
    }
    pub fn gpt4o_mini(&self) -> Model {
        Model { name: "gpt-4o-mini", secret: None }
    }
    pub fn gpt4o(&self) -> Model {
        Model { name: "gpt-4o", secret: None }
    }
}
impl Ollama {
    pub fn gemma2b(&self) -> Model {
        Model { name: "gemma:2b", secret: None }
    }
}
impl Anthropic {
    pub fn claude3_haiku_20240307(&self) -> Model {
        Model { name: "claude-3-haiku-20240307", secret: None }
    }
    pub fn claude3_sonnet_20240229(&self) -> Model {
        Model { name: "claude-3-sonnet-20240229", secret: None }
    }
    pub fn claude3_opus_20240229(&self) -> Model {
        Model { name: "claude-3-opus-20240229", secret: None }
    }
    pub fn claude35_sonnet_20240620(&self) -> Model {
        Model { name: "claude-3-5-sonnet-20240620", secret: None }
    }
}

impl Cohere {
    pub fn command_light_nightly(&self) -> Model {
        Model { name: "command-light-nightly", secret: None }
    }
    pub fn command_light(&self) -> Model {
        Model { name: "command-light", secret: None }
    }
    pub fn command_nightly(&self) -> Model {
        Model { name: "command-nightly", secret: None }
    }
    pub fn command(&self) -> Model {
        Model { name: "command", secret: None }
    }
    pub fn command_r(&self) -> Model {
        Model { name: "command-r", secret: None }
    }
    pub fn command_r_plus(&self) -> Model {
        Model { name: "command-r-plus", secret: None }
    }
}

impl Gemini {
    pub fn gemini15_flash_latest(&self) -> Model {
        Model { name: "gemini-1.5-flash-latest", secret: None }
    }
    pub fn gemini10_pro(&self) -> Model {
        Model { name: "gemini-1.0-pro", secret: None }
    }
    pub fn gemini15_flash(&self) -> Model {
        Model { name: "gemini-1.5-flash", secret: None }
    }
    pub fn gemini15_pro(&self) -> Model {
        Model { name: "gemini-1.5-pro", secret: None }
    }
}

impl Groq {
    pub fn llama708192(&self) -> Model {
        Model { name: "llama3-70b-8192", secret: None }
    }
    pub fn llama38192(&self) -> Model {
        Model { name: "llama3-8b-8192", secret: None }
    }
    pub fn llama_groq8b8192_tool_use_preview(&self) -> Model {
        Model { name: "llama3-groq-8b-8192-tool-use-preview", secret: None }
    }
    pub fn llama_groq70b8192_tool_use_preview(&self) -> Model {
        Model { name: "llama3-groq-70b-8192-tool-use-preview", secret: None }
    }
    pub fn gemma29b_it(&self) -> Model {
        Model { name: "gemma2-9b-it", secret: None }
    }
    pub fn gemma7b_it(&self) -> Model {
        Model { name: "gemma-7b-it", secret: None }
    }
    pub fn mixtral_8x7b32768(&self) -> Model {
        Model { name: "mixtral-8x7b-32768", secret: None }
    }
    pub fn llama8b_instant(&self) -> Model {
        Model { name: "llama-3.1-8b-instant", secret: None }
    }
    pub fn llama70b_versatile(&self) -> Model {
        Model { name: "llama-3.1-70b-versatile", secret: None }
    }
    pub fn llama405b_reasoning(&self) -> Model {
        Model { name: "llama-3.1-405b-reasoning", secret: None }
    }
}
