use std::collections::HashSet;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use super::is_default;

mod remote {
  use std::num::NonZeroU64;

  pub fn default_max_age() -> NonZeroU64 {
    NonZeroU64::new(5 * 60 * 1000).unwrap()
  }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Jwks {
  Const(String),
  File(String),
  #[serde(rename_all = "camelCase")]
  Remote {
    url: String,
    #[serde(default = "remote::default_max_age")]
    max_age: NonZeroU64,
  },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JwtProvider {
  #[serde(skip_serializing_if = "is_default")]
  pub issuer: Option<String>,
  #[serde(default, skip_serializing_if = "is_default")]
  pub audiences: HashSet<String>,
  #[serde(default, skip_serializing_if = "is_default")]
  pub optional_kid: bool,
  pub jwks: Jwks,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum AuthProvider {
  JWT(JwtProvider),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AuthEntry {
  pub id: String,
  #[serde(flatten)]
  pub provider: AuthProvider,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq, Eq)]
pub struct Auth(pub Vec<AuthEntry>);

impl Auth {
  pub fn merge_right(self, other: Auth) -> Self {
    let mut providers = self.0;

    providers.extend(other.0);

    Self(providers)
  }

  pub fn is_some(&self) -> bool {
    !self.0.is_empty()
  }
}

#[cfg(test)]
mod tests {

  use anyhow::Result;
  use serde_json::json;

  use super::*;

  #[test]
  fn jwt_options_parse() -> Result<()> {
    let config: JwtProvider = serde_json::from_value(json!({
      "jwks": {
        "file": "tests/server/config/jwks.json"
      }
    }))?;

    assert!(matches!(
      config,
      JwtProvider { optional_kid: false, jwks: Jwks::File(_), .. }
    ));

    let config: JwtProvider = serde_json::from_value(json!({
      "optionalKid": true,
      "jwks": {
        "remote": {
          "url": "http://localhost:3000"
        }
      }
    }))?;

    assert!(matches!(
      config,
      JwtProvider { optional_kid: true, jwks: Jwks::Remote { .. }, .. }
    ));

    Ok(())
  }
}
