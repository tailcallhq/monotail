use url::Url;

use crate::core::config::Config;
use crate::core::generator::json::ConfigTransformer;
use crate::core::valid::Valid;

pub struct SchemaGenerator {
    query_name: Option<String>,
    url: Option<Url>,
}

impl SchemaGenerator {
    pub fn new(query_name: Option<String>, url: Option<Url>) -> Self {
        Self { query_name, url }
    }

    pub fn generate_schema(&self, config: &mut Config) {
        config.schema.query.clone_from(&self.query_name);
        // TODO: add support for mutations and subscriptions later on.
    }

    pub fn generate_upstream(&self, config: &mut Config) {
        if let Some(url) = &self.url {
            let base_url = match url.host_str() {
                Some(host) => match url.port() {
                    Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
                    None => format!("{}://{}", url.scheme(), host),
                },
                None => url.to_string(),
            };
            config.upstream.base_url = Some(base_url);
        }
    }
}

impl ConfigTransformer for SchemaGenerator {
    fn apply(&mut self, mut config: Config) -> Valid<Config, String> {
        self.generate_schema(&mut config);
        self.generate_upstream(&mut config);

        Valid::succeed(config)
    }
}

#[cfg(test)]
mod test {
    use anyhow::Ok;
    use url::Url;

    use super::SchemaGenerator;
    use crate::core::generator::json::ConfigTransformer;
    use crate::core::valid::Validator;

    #[test]
    fn test_schema_generator_with_query() -> anyhow::Result<()> {
        let mut schema_gen = SchemaGenerator::new(Some("Query".to_string()), None);
        let config = schema_gen.apply(Default::default()).to_result()?;
        insta::assert_snapshot!(config.to_sdl());
        Ok(())
    }

    #[test]
    fn test_schema_generator_without_query() -> anyhow::Result<()> {
        let mut schema_gen = SchemaGenerator::new(None, None);
        let config = schema_gen.apply(Default::default()).to_result()?;
        assert!(config.to_sdl().is_empty());
        Ok(())
    }

    #[test]
    fn test_apply_with_host_and_port() -> anyhow::Result<()> {
        let url = Url::parse("http://example.com:8080").unwrap();
        let mut generator = SchemaGenerator::new(None, Some(url));
        let updated_config = generator.apply(Default::default()).to_result()?;
        assert_eq!(
            updated_config.upstream.base_url,
            Some("http://example.com:8080".to_string())
        );
        Ok(())
    }

    #[test]
    fn test_apply_with_host_without_port() -> anyhow::Result<()> {
        let url = Url::parse("http://example.com").unwrap();
        let mut generator = SchemaGenerator::new(None, Some(url));
        let updated_config = generator.apply(Default::default()).to_result()?;

        assert_eq!(
            updated_config.upstream.base_url,
            Some("http://example.com".to_string())
        );
        Ok(())
    }

    #[test]
    fn test_apply_with_https_scheme() -> anyhow::Result<()> {
        let url = Url::parse("https://example.com").unwrap();
        let mut generator = SchemaGenerator::new(None, Some(url));
        let updated_config = generator.apply(Default::default()).to_result()?;

        assert_eq!(
            updated_config.upstream.base_url,
            Some("https://example.com".to_string())
        );
        Ok(())
    }
}
