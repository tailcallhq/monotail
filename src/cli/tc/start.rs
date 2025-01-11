use anyhow::Result;
use tailcall_enterprise::Enterprise;
use tracing::info;

use super::helpers::log_endpoint_set;
use crate::cli::fmt::Fmt;
use crate::cli::server::Server;
use crate::core::config::reader::ConfigReader;

pub(super) async fn start_command(
    file_paths: Vec<String>,
    config_reader: &ConfigReader,
) -> Result<()> {
    let config_module = config_reader.read_all(&file_paths).await?;
    log_endpoint_set(&config_module.extensions().endpoint_set);
    Fmt::log_n_plus_one(false, config_module.config());

    // config module understand the configuration and the features enabled in the
    // configuration.
    if config_module.is_enterprise_features_enabled() {
        let enterprise = Enterprise::try_new().await?;
        if enterprise.is_validated() {
            info!("TAILCALL_TOKEN validated successfully. Enabling all enterprise features")
        }
    }

    let server = Server::new(config_module);
    server.fork_start().await?;
    Ok(())
}
