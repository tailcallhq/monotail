use std::env;

use machineid_rs::{Encryption, HWIDComponent, IdBuilder};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sysinfo::System;

const API_SECRET: &'static str = "GVaEzXFeRkCI9YBIylbEjQ";
const MEASUREMENT_ID: &'static str = "G-JEP3QDWT0G";
const BASE_URL: &'static str = "https://www.google-analytics.com";
const PARAPHRASE: &'static str = "tc_key";

pub const VERSION: &str = match option_env!("APP_VERSION") {
    Some(version) => version,
    _ => "0.1.0-dev",
};

///
/// Base structure to track usage of the CLI application
#[derive(Debug, Clone)]
pub struct Tracker {
    base_url: String,
    api_secret: String,
    measurement_id: String,
    is_tracking: bool,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
            api_secret: API_SECRET.to_string(),
            measurement_id: MEASUREMENT_ID.to_string(),
            is_tracking: Self::get_usage_tracking(),
        }
    }

    pub async fn init_ping(&'static self) {
        if self.is_tracking {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            tokio::task::spawn(async move {
                loop {
                    interval.tick().await;
                    let _ = self.dispatch("ping".to_string()).await;
                }
            });
        }
    }

    fn create_request(&self, event_name: String) -> anyhow::Result<reqwest::Request> {
        let event = EventList::prepare_event(event_name)?;
        tracing::debug!("Sending event: {:?}", event);
        let mut url = reqwest::Url::parse(self.base_url.as_str())?;
        url.set_path("/mp/collect");
        url.query_pairs_mut()
            .append_pair("api_secret", self.api_secret.as_str())
            .append_pair("measurement_id", self.measurement_id.as_str());
        let mut request = reqwest::Request::new(reqwest::Method::POST, url);
        let header_name = HeaderName::from_static("content-type");
        let header_value = HeaderValue::from_str("application/json")?;
        request.headers_mut().insert(header_name, header_value);
        let event = serde_json::json!({
            "client_id": event.client_id,
            "events": event.events,
        });

        let _ = request
            .body_mut()
            .insert(reqwest::Body::from(serde_json::to_string(&event)?));
        Ok(request)
    }

    pub async fn send_request(request: reqwest::Request) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let response = client.execute(request).await?;
        let status = response.status();
        let text = response.text().await?;
        tracing::debug!("Tracker: {}, message: {:?}", status.as_str(), text);
        Ok(())
    }

    pub async fn dispatch(&'static self, event_name: String) -> anyhow::Result<()> {
        if self.is_tracking {
            let request = self.create_request(event_name)?;
            Self::send_request(request).await?;
            Ok(())
        } else {
            Ok(())
        }
    }
    fn get_usage_tracking() -> bool {
        const LONG_ENV_FILTER_VAR_NAME: &str = "TAILCALL_TRACKER";
        const SHORT_ENV_FILTER_VAR_NAME: &str = "TC_TRACKER";

        let is_prod = !VERSION.contains("dev");

        let usage_enabled = env::var(LONG_ENV_FILTER_VAR_NAME)
            .or(env::var(SHORT_ENV_FILTER_VAR_NAME))
            .map(|v| !v.eq_ignore_ascii_case("false"))
            .ok();
        Tracker::usage_tracking_inner(is_prod, usage_enabled)
    }

    fn usage_tracking_inner(is_prod: bool, usage_enabled: Option<bool>) -> bool {
        if let Some(usage_enabled) = usage_enabled {
            usage_enabled
        } else {
            is_prod
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Params {
    pub cpu_cores: String,
    pub os_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Event {
    pub name: String,
    pub params: Params,
}

#[derive(Debug, Serialize, Deserialize)]
struct EventList {
    pub client_id: String,
    pub events: Vec<Event>,
}

impl EventList {
    fn create_client_id() -> anyhow::Result<String> {
        let mut builder = IdBuilder::new(Encryption::SHA256);
        builder
            .add_component(HWIDComponent::SystemID)
            .add_component(HWIDComponent::CPUCores);

        Ok(builder.build(PARAPHRASE)?)
    }
    fn prepare_event(command_name: String) -> anyhow::Result<EventList> {
        let sys = System::new_all();
        let cores = sys.physical_core_count().unwrap_or(2).to_string();
        let os_name = System::long_os_version().unwrap_or("Unknown".to_string());
        Ok(EventList {
            client_id: EventList::create_client_id()?,
            events: vec![Event {
                name: command_name,
                params: Params { cpu_cores: cores, os_name },
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_enabled_true() {
        assert!(Tracker::usage_tracking_inner(true, Some(true)));
        assert!(Tracker::usage_tracking_inner(false, Some(true)));
    }

    #[test]
    fn usage_enabled_false() {
        assert!(!Tracker::usage_tracking_inner(true, Some(false)));
        assert!(!Tracker::usage_tracking_inner(false, Some(false)));
    }

    #[test]
    fn usage_enabled_none_is_prod_true() {
        assert!(Tracker::usage_tracking_inner(true, None));
    }

    #[test]
    fn usage_enabled_none_is_prod_false() {
        assert!(!Tracker::usage_tracking_inner(false, None));
    }
}
