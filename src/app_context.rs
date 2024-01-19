use std::sync::Arc;

use async_graphql::dynamic::{self, DynamicRequest};
use async_graphql::Response;
use async_graphql_value::ConstValue;

use crate::auth::context::GlobalAuthContext;
use crate::blueprint::Type::ListType;
use crate::blueprint::{Blueprint, Definition};
use crate::data_loader::DataLoader;
use crate::graphql::GraphqlDataLoader;
use crate::grpc::data_loader::GrpcDataLoader;
use crate::http::{DataLoaderRequest, HttpDataLoader};
use crate::lambda::{DataLoaderId, Expression, IO};
use crate::{grpc, Cache, EnvIO, HttpIO};

pub struct AppContext<Http, Env> {
  pub schema: dynamic::Schema,
  pub universal_http_client: Arc<Http>,
  pub http2_only_client: Arc<Http>,
  pub blueprint: Blueprint,
  pub http_data_loaders: Arc<Vec<DataLoader<DataLoaderRequest, HttpDataLoader>>>,
  pub gql_data_loaders: Arc<Vec<DataLoader<DataLoaderRequest, GraphqlDataLoader>>>,
  pub cache: ChronoCache<u64, ConstValue>,
  pub grpc_data_loaders: Arc<Vec<DataLoader<grpc::DataLoaderRequest, GrpcDataLoader>>>,
  pub cache: ChronoCache<u64, ConstValue>,
  pub env_vars: Arc<Env>,
  pub auth_ctx: Arc<GlobalAuthContext>,
}

impl<Http: HttpIO, Env: EnvIO> AppContext<Http, Env> {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    mut blueprint: Blueprint,
    h_client: Arc<Http>,
    h2_client: Arc<Http>,
    env: Arc<Env>,
    chrono_cache: Arc<dyn Cache<u64, ConstValue>>,
  ) -> Self {
    let mut http_data_loaders = vec![];
    let mut gql_data_loaders = vec![];
    let mut grpc_data_loaders = vec![];

    for def in blueprint.definitions.iter_mut() {
      if let Definition::ObjectTypeDefinition(def) = def {
        for field in &mut def.fields {
          if let Some(Expression::IO(expr)) = &mut field.resolver {
            match expr {
              IO::Http { req_template, group_by, .. } => {
                let data_loader = HttpDataLoader::new(
                  h_client.clone(),
                  group_by.clone(),
                  matches!(&field.of_type, ListType { .. }),
                )
                .to_data_loader(blueprint.upstream.batch.clone().unwrap_or_default());

                field.resolver = Some(Expression::IO(IO::Http {
                  req_template: req_template.clone(),
                  group_by: group_by.clone(),
                  dl_id: Some(DataLoaderId(http_data_loaders.len())),
                }));

                http_data_loaders.push(data_loader);
              }

              IO::GraphQLEndpoint { req_template, field_name, batch, .. } => {
                let graphql_data_loader = GraphqlDataLoader::new(h_client.clone(), *batch)
                  .to_data_loader(blueprint.upstream.batch.clone().unwrap_or_default());

                field.resolver = Some(Expression::IO(IO::GraphQLEndpoint {
                  req_template: req_template.clone(),
                  field_name: field_name.clone(),
                  batch: *batch,
                  dl_id: Some(DataLoaderId(gql_data_loaders.len())),
                }));

                gql_data_loaders.push(graphql_data_loader);
              }

              IO::Grpc { req_template, group_by, .. } => {
                let data_loader = GrpcDataLoader {
                  client: h2_client.clone(),
                  operation: req_template.operation.clone(),
                  group_by: group_by.clone(),
                };
                let data_loader = data_loader.to_data_loader(blueprint.upstream.batch.clone().unwrap_or_default());

                field.resolver = Some(Expression::IO(IO::Grpc {
                  req_template: req_template.clone(),
                  group_by: group_by.clone(),
                  dl_id: Some(DataLoaderId(grpc_data_loaders.len())),
                }));

                grpc_data_loaders.push(data_loader);
              }
              _ => {}
            }
          }
        }
      }
    }

    let schema = blueprint.to_schema();

    let auth = blueprint.server.auth.clone();

    let auth_ctx = GlobalAuthContext::new(auth, h_client.clone());

    AppContext {
      schema,
      universal_http_client: h_client,
      http2_only_client: h2_client,
      blueprint,
      http_data_loaders: Arc::new(http_data_loaders),
      gql_data_loaders: Arc::new(gql_data_loaders),
      cache: chrono_cache,
      grpc_data_loaders: Arc::new(grpc_data_loaders),
      env_vars: env,
      auth_ctx: Arc::new(auth_ctx),
    }
  }

  pub async fn execute(&self, request: impl Into<DynamicRequest>) -> Response {
    self.schema.execute(request).await
  }
}
