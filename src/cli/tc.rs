use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use clap::Parser;
use convert_case::{Case, Casing};
use dotenvy::dotenv;
use inquire::{Confirm, Select, Text};
use lazy_static::lazy_static;
use stripmargin::StripMargin;

use super::command::{Cli, Command};
use super::update_checker;
use crate::cli::fmt::Fmt;
use crate::cli::server::Server;
use crate::cli::{self, CLIError};
use crate::core::blueprint::Blueprint;
use crate::core::config::reader::ConfigReader;
use crate::core::config::{Arg, Config, Field, RootSchema, Source, Type};
use crate::core::generator::Generator;
use crate::core::http::API_URL_PREFIX;
use crate::core::print_schema;
use crate::core::rest::{EndpointSet, Unchecked};

const FILE_NAME: &str = ".tailcallrc.graphql";
const YML_FILE_NAME: &str = ".graphqlrc.yml";
const JSON_FILE_NAME: &str = ".tailcallrc.schema.json";

lazy_static! {
    static ref TRACKER: tailcall_tracker::Tracker = tailcall_tracker::Tracker::default();
}
pub async fn run() -> Result<()> {
    if let Ok(path) = dotenv() {
        tracing::info!("Env file: {:?} loaded", path);
    }
    let cli = Cli::parse();
    update_checker::check_for_update().await;
    let runtime = cli::runtime::init(&Blueprint::default());
    let config_reader = ConfigReader::init(runtime.clone());

    // Initialize ping event every 60 seconds
    let _ = TRACKER
        .init_ping(tokio::time::Duration::from_secs(60))
        .await;

    // Dispatch the command as an event
    let _ = TRACKER
        .dispatch(cli.command.to_string().to_case(Case::Snake).as_str())
        .await;
    match cli.command {
        Command::Start { file_paths } => {
            let config_module = config_reader.read_all(&file_paths).await?;
            log_endpoint_set(&config_module.extensions.endpoint_set);
            Fmt::log_n_plus_one(false, &config_module.config);
            let server = Server::new(config_module);
            server.fork_start().await?;
            Ok(())
        }
        Command::Check { file_paths, n_plus_one_queries, schema, format } => {
            let config_module = (config_reader.read_all(&file_paths)).await?;
            log_endpoint_set(&config_module.extensions.endpoint_set);
            if let Some(format) = format {
                Fmt::display(format.encode(&config_module)?);
            }
            let blueprint = Blueprint::try_from(&config_module).map_err(CLIError::from);

            match blueprint {
                Ok(blueprint) => {
                    tracing::info!("Config {} ... ok", file_paths.join(", "));
                    Fmt::log_n_plus_one(n_plus_one_queries, &config_module.config);
                    // Check the endpoints' schema
                    let _ = config_module
                        .extensions
                        .endpoint_set
                        .into_checked(&blueprint, runtime)
                        .await?;
                    if schema {
                        display_schema(&blueprint);
                    }
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        }
        Command::Init { folder_path } => init(&folder_path).await,
        Command::Gen { paths, input, output, query } => {
            let generator = Generator::init(runtime);
            let cfg = generator
                .read_all(input, paths.as_ref(), query.as_str())
                .await?;

            let config = output.unwrap_or_default().encode(&cfg)?;
            Fmt::display(config);
            Ok(())
        }
    }
}

pub async fn init(folder_path: &str) -> Result<()> {
    let project_name = Text::new("Project Name:").prompt().unwrap_or_else(|err| {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    });

    let chosen_format = Select::new(
        "Choose a file format:",
        vec![Source::GraphQL, Source::Json, Source::Yml],
    )
    .prompt()
    .unwrap_or_else(|err| {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    });

    let folder_exists = fs::metadata(folder_path).is_ok();

    if !folder_exists {
        let confirm = Confirm::new(&format!(
            "Do you want to create the folder {}?",
            folder_path
        ))
        .with_default(false)
        .prompt()?;

        if confirm {
            fs::create_dir_all(folder_path)?;
        } else {
            return Ok(());
        };
    }

    let tailcallrc = include_str!("../../generated/.tailcallrc.graphql");
    let tailcallrc_json: &str = include_str!("../../generated/.tailcallrc.schema.json");

    let file_path = Path::new(folder_path).join(FILE_NAME);
    let json_file_path = Path::new(folder_path).join(JSON_FILE_NAME);
    let yml_file_path = Path::new(folder_path).join(YML_FILE_NAME);

    let tailcall_exists = fs::metadata(&file_path).is_ok();

    if tailcall_exists {
        // confirm overwrite
        let confirm = Confirm::new(&format!("Do you want to overwrite the file {}?", FILE_NAME))
            .with_default(false)
            .prompt()?;

        if confirm {
            fs::write(&file_path, tailcallrc.as_bytes())?;
            fs::write(&json_file_path, tailcallrc_json.as_bytes())?;
        }
    } else {
        fs::write(&file_path, tailcallrc.as_bytes())?;
        fs::write(&json_file_path, tailcallrc_json.as_bytes())?;
    }

    let yml_exists = fs::metadata(&yml_file_path).is_ok();

    if !yml_exists {
        fs::write(&yml_file_path, "")?;

        let graphqlrc = r"|schema:
         |- './.tailcallrc.graphql'
    "
        .strip_margin();

        fs::write(&yml_file_path, graphqlrc)?;
    }

    let graphqlrc = fs::read_to_string(&yml_file_path)?;

    let file_path = file_path.to_str().unwrap();

    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&graphqlrc)?;

    if let Some(mapping) = yaml.as_mapping_mut() {
        let schema = mapping
            .entry("schema".into())
            .or_insert(serde_yaml::Value::Sequence(Default::default()));
        if let Some(schema) = schema.as_sequence_mut() {
            if !schema
                .iter()
                .any(|v| v == &serde_yaml::Value::from("./.tailcallrc.graphql"))
            {
                let confirm =
                    Confirm::new(&format!("Do you want to add {} to the schema?", file_path))
                        .with_default(false)
                        .prompt()?;

                if confirm {
                    schema.push(serde_yaml::Value::from("./.tailcallrc.graphql"));
                    let updated = serde_yaml::to_string(&yaml)?;
                    fs::write(yml_file_path, updated)?;
                }
            }
        }
    }

    let file = format!("{}.{}", project_name, chosen_format);
    let file_path = Path::new(folder_path).join(file);
    let config = hello_world_config();
    let content = chosen_format.encode(&config)?;
    fs::write(&file_path, content)?;
    tracing::info!("Created file: {}", file_path.to_str().unwrap_or_default());

    Ok(())
}

fn hello_world_config() -> Config {
    let mut config = Config::default();

    config.server.hostname = Some("localhost".to_string());
    config.server.port = Some(8000);
    config.schema = RootSchema::default().query("Query".to_string());

    let arg = Arg {
        type_of: "String".to_string(),
        default_value: Some(serde_json::json!("World")),
        ..Default::default()
    };

    let field = Field {
        type_of: "String".to_string(),
        required: true,
        args: BTreeMap::from_iter(vec![("name".to_string(), arg)]),
        ..Default::default()
    };

    let ty = Type {
        fields: BTreeMap::from_iter(vec![("hello".to_string(), field)]),
        ..Default::default()
    };
    config.types.insert("Query".to_string(), ty);

    config
}

fn log_endpoint_set(endpoint_set: &EndpointSet<Unchecked>) {
    let mut endpoints = endpoint_set.get_endpoints().clone();
    endpoints.sort_by(|a, b| {
        let method_a = a.get_method();
        let method_b = b.get_method();
        if method_a.eq(method_b) {
            a.get_path().as_str().cmp(b.get_path().as_str())
        } else {
            method_a.to_string().cmp(&method_b.to_string())
        }
    });
    for endpoint in endpoints {
        tracing::info!(
            "Endpoint: {} {}{} ... ok",
            endpoint.get_method(),
            API_URL_PREFIX,
            endpoint.get_path().as_str()
        );
    }
}

pub fn display_schema(blueprint: &Blueprint) {
    Fmt::display(Fmt::heading("GraphQL Schema:\n"));
    let sdl = blueprint.to_schema();
    Fmt::display(format!("{}\n", print_schema::print_schema(sdl)));
}
