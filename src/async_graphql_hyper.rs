use std::any::Any;
use std::collections::BTreeMap;

use anyhow::Result;
use async_graphql::{BatchResponse, Executor};
use hyper::header::{HeaderName, HeaderValue, CACHE_CONTROL, CONTENT_TYPE};
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct GraphQLBatchRequest(pub async_graphql::BatchRequest);
impl GraphQLBatchRequest {
  /// Shortcut method to execute the request on the executor.
  pub async fn execute<E>(self, executor: &E) -> GraphQLResponse
  where
    E: Executor,
  {
    GraphQLResponse(executor.execute_batch(self.0).await)
  }
}
#[derive(Debug, Deserialize)]
pub struct GraphQLRequest(pub async_graphql::Request);

impl GraphQLRequest {
  /// Shortcut method to execute the request on the schema.
  pub async fn execute<E>(self, executor: &E) -> GraphQLResponse
  where
    E: Executor,
  {
    GraphQLResponse(executor.execute(self.0).await.into())
  }

  /// Insert some data for this request.
  #[must_use]
  pub fn data<D: Any + Send + Sync>(mut self, data: D) -> Self {
    self.0.data.insert(data);
    self
  }
}
#[derive(Debug, Serialize)]
pub struct GraphQLResponse(pub async_graphql::BatchResponse);
impl From<async_graphql::BatchResponse> for GraphQLResponse {
  fn from(batch: async_graphql::BatchResponse) -> Self {
    Self(batch)
  }
}
impl From<async_graphql::Response> for GraphQLResponse {
  fn from(res: async_graphql::Response) -> Self {
    Self(res.into())
  }
}

impl From<GraphQLQuery> for GraphQLRequest {
  fn from(query: GraphQLQuery) -> Self {
    let mut request = async_graphql::Request::new(query.query);

    if let Some(operation_name) = query.operation_name {
      request = request.operation_name(operation_name);
    }

    if let Some(variables) = query.variables {
      let value = serde_json::from_str(&variables).unwrap_or_default();
      let variables = async_graphql::Variables::from_json(value);
      request = request.variables(variables);
    }

    GraphQLRequest(request)
  }
}

#[derive(Debug)]
pub struct GraphQLQuery {
  query: String,
  operation_name: Option<String>,
  variables: Option<String>,
}

impl GraphQLQuery {
  /// Shortcut method to execute the request on the schema.
  pub async fn execute<E>(self, executor: &E) -> GraphQLResponse
  where
    E: Executor,
  {
    let request: GraphQLRequest = self.into();
    request.execute(executor).await
  }
}

lazy_static::lazy_static! {
  static ref APPLICATION_JSON:HeaderValue = HeaderValue::from_static("application/json");
}

impl GraphQLResponse {
  pub fn to_response(self, headers: Option<BTreeMap<String, String>>) -> Result<Response<hyper::Body>> {
    let mut response = Response::builder()
      .status(StatusCode::OK)
      .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
      .body(Body::from(serde_json::to_string(&self.0)?))?;

    if let Some(headers) = headers {
      for (k, v) in headers {
        response.headers_mut().insert(
          HeaderName::from_bytes(k.as_bytes())?,
          HeaderValue::from_bytes(v.as_bytes())?,
        );
      }
    }

    if self.0.is_ok() {
      if let Some(cache_control) = self.0.cache_control().value() {
        response
          .headers_mut()
          .insert(CACHE_CONTROL, HeaderValue::from_str(cache_control.as_str())?);
      }
    }

    Ok(response)
  }

  /// Sets the `cache_control` max_age for a given `GraphQLResponse`.
  ///
  /// The function modifies the `GraphQLResponse` to set the `cache_control` `max_age`
  /// to the specified `min_cache` value.
  ///
  /// # Arguments
  ///
  /// * `res` - The GraphQL response whose `cache_control` is to be set.
  /// * `min_cache` - The `max_age` value to be set for `cache_control`.
  ///
  /// # Returns
  ///
  /// * A modified `GraphQLResponse` with updated `cache_control` `max_age`.
  pub fn set_cache_control(mut self, min_cache: i32) -> GraphQLResponse {
    match self.0 {
      BatchResponse::Single(ref mut res) => {
        res.cache_control.max_age = min_cache;
      }
      BatchResponse::Batch(ref mut list) => {
        for res in list {
          res.cache_control.max_age = min_cache;
        }
      }
    };
    self
  }
}
