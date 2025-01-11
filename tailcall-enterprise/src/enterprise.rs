#![allow(dead_code)]
use std::env;

use keygen_rs::config::KeygenConfig;
use keygen_rs::errors::Error;
use postgrest::Postgrest;
use serde::Deserialize;
use thiserror::Error;

const TOKEN_NAME: &str = "TAILCALL_TOKEN";

// Supabase configurations
const TABLE_NAME: &str = "---Add-Table-Name-Here---";
const ANON_KEY: &str = "---Add-Anonymous-Key-Here---";
const API_URL: &str = "---Add-Api-Url-Here---";

#[derive(Error, Debug)]
pub enum EnterpriseError {
    #[error("TAILCALL_TOKEN is not provided. Please connect via https://tailcall.run/contact/ if you haven't already got a token.")]
    TokenNotProvided,
    #[error("Failed to fetch public key: {0}")]
    PublicKeyFetchError(String),
    #[error("Failed to parse public key: {0}")]
    PublicKeyParsingError(String),
    #[error(transparent)]
    KeygenError(#[from] Box<Error>),
    #[error(transparent)]
    DatabaseClientError(#[from] reqwest::Error),
}

#[derive(Debug)]
pub struct Enterprise {
    license_key: Option<String>,
    config: Option<KeygenConfig>,
}

#[derive(Deserialize, Debug)]
struct KeygenData {
    public_key: String,
    account: String,
    product: String,
    api_url: String,
    api_version: String,
    api_prefix: String,
}

impl From<KeygenData> for KeygenConfig {
    fn from(data: KeygenData) -> Self {
        KeygenConfig {
            public_key: Some(data.public_key),
            account: data.account,
            product: data.product,
            api_url: data.api_url,
            api_version: data.api_version,
            api_prefix: data.api_prefix,
            ..Default::default()
        }
    }
}

impl Enterprise {
    pub fn is_validated(&self) -> bool {
        self.config.is_some() && self.license_key.is_some()
    }

    pub async fn try_new() -> Result<Self, EnterpriseError> {
        match env::var(TOKEN_NAME) {
            Ok(signed_key) => {
                let keygen_data = Self::fetch_public_key().await?;
                let mut config: KeygenConfig = keygen_data.into();
                config.license_key = Some(signed_key.clone());
                keygen_rs::config::set_config(config);
                let _signed_key_result =
                    keygen_rs::verify(keygen_rs::license::SchemeCode::Ed25519Sign, &signed_key)
                        .map_err(|e| match e {
                            Error::LicenseKeyMissing => EnterpriseError::TokenNotProvided,
                            _ => EnterpriseError::KeygenError(Box::new(e)),
                        })?;
                Ok(Self {
                    license_key: Some(signed_key),
                    config: Some(keygen_rs::config::get_config()),
                })
            }
            Err(_) => Err(EnterpriseError::TokenNotProvided),
        }
    }

    async fn fetch_public_key() -> Result<KeygenData, EnterpriseError> {
        let client = Postgrest::new(API_URL).insert_header("apiKey", ANON_KEY);
        // pick the latest key from the database.
        let result = client
            .from(TABLE_NAME)
            .select("*")
            .order("id.desc")
            .limit(1)
            .single()
            .execute()
            .await?;
        let body = result.json::<KeygenData>().await?;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[tokio::test]
    async fn test_no_token_provided() {
        env::remove_var(TOKEN_NAME);
        let result = Enterprise::try_new().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_token() {
        env::set_var(TOKEN_NAME, "invalid-token");
        let result = Enterprise::try_new().await;
        assert!(result.is_err());
    }
}
