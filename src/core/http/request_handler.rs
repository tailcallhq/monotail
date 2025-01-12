use std::collections::BTreeSet;
use std::ops::Deref;
use std::sync::Arc;

use anyhow::Result;
use async_graphql::ServerError;
use hyper::header::{self, HeaderValue, CONTENT_TYPE};
use hyper::http::request::Parts;
use hyper::http::Method;
use hyper::{Body, HeaderMap, Request, Response, StatusCode};
use opentelemetry::trace::SpanKind;
use opentelemetry_semantic_conventions::trace::{HTTP_REQUEST_METHOD, HTTP_ROUTE};
use prometheus::{Encoder, ProtobufEncoder, TextEncoder, TEXT_FORMAT};
use serde::de::DeserializeOwned;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::request_context::RequestContext;
use super::telemetry::{get_response_status_code, RequestCounter};
use super::{showcase, telemetry, TAILCALL_HTTPS_ORIGIN, TAILCALL_HTTP_ORIGIN};
use crate::core::app_context::AppContext;
use crate::core::async_graphql_hyper::{GraphQLRequestLike, GraphQLResponse};
use crate::core::blueprint::telemetry::TelemetryExporter;
use crate::core::config::{PrometheusExporter, PrometheusFormat};
use crate::core::jit::JITExecutor;

pub const API_URL_PREFIX: &str = "/api";

fn prometheus_metrics(prometheus_exporter: &PrometheusExporter) -> Result<Response<Body>> {
    let metric_families = prometheus::default_registry().gather();
    let mut buffer = vec![];

    match prometheus_exporter.format {
        PrometheusFormat::Text => TextEncoder::new().encode(&metric_families, &mut buffer)?,
        PrometheusFormat::Protobuf => {
            ProtobufEncoder::new().encode(&metric_families, &mut buffer)?
        }
    };

    let content_type = match prometheus_exporter.format {
        PrometheusFormat::Text => TEXT_FORMAT,
        PrometheusFormat::Protobuf => prometheus::PROTOBUF_FORMAT,
    };

    Ok(Response::builder()
        .status(200)
        .header(CONTENT_TYPE, content_type)
        .body(Body::from(buffer))?)
}

fn not_found() -> Result<Response<Body>> {
    Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())?)
}

fn create_request_context(req: &Request<Body>, app_ctx: &AppContext) -> RequestContext {
    let allowed_headers =
        create_allowed_headers(req.headers(), &app_ctx.blueprint.upstream.allowed_headers);
    RequestContext::from(app_ctx).allowed_headers(allowed_headers)
}

pub fn update_response_headers(
    resp: &mut Response<Body>,
    req_ctx: &RequestContext,
    app_ctx: &AppContext,
) {
    if !app_ctx.blueprint.server.response_headers.is_empty() {
        // Add static response headers
        resp.headers_mut()
            .extend(app_ctx.blueprint.server.response_headers.clone());
    }

    // Insert Cookie Headers
    if let Some(ref cookie_headers) = req_ctx.cookie_headers {
        let cookie_headers = cookie_headers.lock().unwrap();
        resp.headers_mut().extend(cookie_headers.deref().clone());
    }

    // Insert Experimental Headers
    req_ctx.extend_x_headers(resp.headers_mut());
}

#[tracing::instrument(skip_all, fields(otel.name = "graphQL", otel.kind = ?SpanKind::Server))]
pub async fn graphql_request<T: DeserializeOwned + GraphQLRequestLike>(
    req: Request<Body>,
    app_ctx: &Arc<AppContext>,
    req_counter: &mut RequestCounter,
) -> Result<Response<Body>> {
    req_counter.set_http_route("/graphql");
    let req_ctx = Arc::new(create_request_context(&req, app_ctx));
    let (req, body) = req.into_parts();
    let bytes = hyper::body::to_bytes(body).await?;
    let graphql_request = serde_json::from_slice::<T>(&bytes);
    match graphql_request {
        Ok(request) => {
            let resp = execute_query(app_ctx, &req_ctx, request, req).await?;
            Ok(resp)
        }
        Err(err) => {
            tracing::error!(
                "Failed to parse request: {}",
                String::from_utf8(bytes.to_vec()).unwrap()
            );

            let mut response = async_graphql::Response::default();
            let server_error =
                ServerError::new(format!("Unexpected GraphQL Request: {}", err), None);
            response.errors = vec![server_error];

            Ok(GraphQLResponse::from(response).into_response()?)
        }
    }
}

async fn execute_query<T: DeserializeOwned + GraphQLRequestLike>(
    app_ctx: &Arc<AppContext>,
    req_ctx: &Arc<RequestContext>,
    request: T,
    req: Parts,
) -> anyhow::Result<Response<Body>> {
    let operation_id = request.operation_id(&req.headers);
    let exec = JITExecutor::new(app_ctx.clone(), req_ctx.clone(), operation_id);
    let mut response = request
        .execute_with_jit(exec)
        .await
        .set_cache_control(
            app_ctx.blueprint.server.enable_cache_control_header,
            req_ctx.get_min_max_age().unwrap_or(0),
            req_ctx.is_cache_public().unwrap_or(true),
        )
        .into_response()?;

    update_response_headers(&mut response, req_ctx, app_ctx);
    Ok(response)
}

fn create_allowed_headers(headers: &HeaderMap, allowed: &BTreeSet<String>) -> HeaderMap {
    let mut new_headers = HeaderMap::with_capacity(allowed.len());
    for (k, v) in headers.iter() {
        if allowed
            .iter()
            .any(|allowed_key| allowed_key.eq_ignore_ascii_case(k.as_str()))
        {
            new_headers.insert(k, v.clone());
        }
    }
    new_headers
}

async fn handle_origin_tailcall<T: DeserializeOwned + GraphQLRequestLike>(
    req: Request<Body>,
    app_ctx: Arc<AppContext>,
    request_counter: &mut RequestCounter,
) -> Result<Response<Body>> {
    let method = req.method();
    if method == Method::OPTIONS {
        let mut res = Response::new(Body::default());
        res.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("https://tailcall.run"),
        );
        res.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        res.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("*"),
        );
        Ok(res)
    } else {
        let mut res = handle_request_inner::<T>(req, app_ctx, request_counter).await?;
        res.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("https://tailcall.run"),
        );

        Ok(res)
    }
}

async fn handle_request_with_cors<T: DeserializeOwned + GraphQLRequestLike>(
    req: Request<Body>,
    app_ctx: Arc<AppContext>,
    request_counter: &mut RequestCounter,
) -> Result<Response<Body>> {
    // Safe to call `.unwrap()` because this method will only be called when
    // `cors` is `Some`
    let cors = app_ctx.blueprint.server.cors.as_ref().unwrap();
    let (parts, body) = req.into_parts();
    let origin = parts.headers.get(&header::ORIGIN);

    let mut headers = HeaderMap::new();

    // These headers are applied to both preflight and subsequent regular CORS
    // requests: https://fetch.spec.whatwg.org/#http-responses

    headers.extend(cors.allow_origin_to_header(origin));
    headers.extend(cors.allow_credentials_to_header());
    headers.extend(cors.allow_private_network_to_header(&parts));
    headers.extend(cors.vary_to_header());

    // Return results immediately upon preflight request
    if parts.method == Method::OPTIONS {
        // These headers are applied only to preflight requests
        headers.extend(cors.allow_methods_to_header());
        headers.extend(cors.allow_headers_to_header());
        headers.extend(cors.max_age_to_header());

        let mut response = Response::new(Body::default());
        std::mem::swap(response.headers_mut(), &mut headers);

        Ok(response)
    } else {
        // This header is applied only to non-preflight requests
        headers.extend(cors.expose_headers_to_header());

        let req = Request::from_parts(parts, body);
        let mut response = handle_request_inner::<T>(req, app_ctx, request_counter).await?;

        let response_headers = response.headers_mut();

        // vary header can have multiple values, don't overwrite
        // previously-set value(s).
        if let Some(vary) = headers.remove(header::VARY) {
            response_headers.append(header::VARY, vary);
        }
        // extend will overwrite previous headers of remaining names
        response_headers.extend(headers.drain());

        Ok(response)
    }
}

async fn handle_rest_apis(
    mut request: Request<Body>,
    app_ctx: Arc<AppContext>,
    req_counter: &mut RequestCounter,
) -> Result<Response<Body>> {
    *request.uri_mut() = request.uri().path().replace(API_URL_PREFIX, "").parse()?;
    let req_ctx = Arc::new(create_request_context(&request, app_ctx.as_ref()));
    if let Some(p_request) = app_ctx.endpoints.matches(&request) {
        let (req, body) = request.into_parts();
        let http_route = format!("{API_URL_PREFIX}{}", p_request.path.as_str());
        req_counter.set_http_route(&http_route);
        let span = tracing::info_span!(
            "REST",
            otel.name = format!("REST {} {}", req.method, p_request.path.as_str()),
            otel.kind = ?SpanKind::Server,
            { HTTP_REQUEST_METHOD } = %req.method,
            { HTTP_ROUTE } = http_route
        );
        return async {
            let graphql_request = p_request.into_request(body).await?;
            let operation_id = graphql_request.operation_id(&req.headers);
            let exec = JITExecutor::new(app_ctx.clone(), req_ctx.clone(), operation_id)
                .flatten_response(true);
            let mut response = graphql_request
                .execute_with_jit(exec)
                .await
                .set_cache_control(
                    app_ctx.blueprint.server.enable_cache_control_header,
                    req_ctx.get_min_max_age().unwrap_or(0),
                    req_ctx.is_cache_public().unwrap_or(true),
                )
                .into_rest_response()?;
            update_response_headers(&mut response, &req_ctx, &app_ctx);
            Ok(response)
        }
        .instrument(span)
        .await;
    }

    not_found()
}

async fn handle_request_inner<T: DeserializeOwned + GraphQLRequestLike>(
    req: Request<Body>,
    app_ctx: Arc<AppContext>,
    req_counter: &mut RequestCounter,
) -> Result<Response<Body>> {
    if req.uri().path().starts_with(API_URL_PREFIX) {
        return handle_rest_apis(req, app_ctx, req_counter).await;
    }

    let health_check_endpoint = app_ctx.blueprint.server.routes.status();
    let graphql_endpoint = app_ctx.blueprint.server.routes.graphql();

    match *req.method() {
        // NOTE:
        // The first check for the route should be for `/graphql`
        // This is always going to be the most used route.
        Method::POST if req.uri().path() == graphql_endpoint => {
            graphql_request::<T>(req, &app_ctx, req_counter).await
        }
        Method::POST
            if app_ctx.blueprint.server.enable_showcase
                && req.uri().path() == "/showcase/graphql" =>
        {
            let app_ctx =
                match showcase::create_app_ctx::<T>(&req, app_ctx.runtime.clone(), false).await? {
                    Ok(app_ctx) => app_ctx,
                    Err(res) => return Ok(res),
                };

            graphql_request::<T>(req, &Arc::new(app_ctx), req_counter).await
        }
        Method::GET if req.uri().path() == health_check_endpoint => {
            let status_response = Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"message": "ready"}"#))?;
            Ok(status_response)
        }
        Method::GET => {
            if let Some(TelemetryExporter::Prometheus(prometheus)) =
                app_ctx.blueprint.telemetry.export.as_ref()
            {
                if req.uri().path() == prometheus.path {
                    return prometheus_metrics(prometheus);
                }
            };
            not_found()
        }
        _ => not_found(),
    }
}

#[tracing::instrument(
    skip_all,
    err,
    fields(
        otel.name = "request",
        otel.kind = ?SpanKind::Server,
        url.path = %req.uri().path(),
        http.request.method = %req.method()
    )
)]
pub async fn handle_request<T: DeserializeOwned + GraphQLRequestLike>(
    req: Request<Body>,
    app_ctx: Arc<AppContext>,
) -> Result<Response<Body>> {
    telemetry::propagate_context(&req);
    let mut req_counter = RequestCounter::new(&app_ctx.blueprint.telemetry, &req);

    let response = if app_ctx.blueprint.server.cors.is_some() {
        handle_request_with_cors::<T>(req, app_ctx, &mut req_counter).await
    } else if let Some(origin) = req.headers().get(&header::ORIGIN) {
        if origin == TAILCALL_HTTPS_ORIGIN || origin == TAILCALL_HTTP_ORIGIN {
            handle_origin_tailcall::<T>(req, app_ctx, &mut req_counter).await
        } else {
            handle_request_inner::<T>(req, app_ctx, &mut req_counter).await
        }
    } else {
        handle_request_inner::<T>(req, app_ctx, &mut req_counter).await
    };

    req_counter.update(&response);
    if let Ok(response) = &response {
        let status = get_response_status_code(response);
        tracing::Span::current().set_attribute(status.key, status.value);
    };

    response
}

#[cfg(test)]
mod test {
    use tailcall_valid::Validator;

    use super::*;
    use crate::core::async_graphql_hyper::GraphQLRequest;
    use crate::core::blueprint::Blueprint;
    use crate::core::config::{Config, ConfigModule, Routes};
    use crate::core::rest::EndpointSet;
    use crate::core::runtime::test::init;

    #[tokio::test]
    async fn test_health_endpoint() -> anyhow::Result<()> {
        let sdl = tokio::fs::read_to_string(tailcall_fixtures::configs::JSONPLACEHOLDER).await?;
        let config = Config::from_sdl(&sdl).to_result()?;
        let mut blueprint = Blueprint::try_from(&ConfigModule::from(config))?;
        blueprint.server.routes = Routes::default().with_status("/health");
        let app_ctx = Arc::new(AppContext::new(
            blueprint,
            init(None),
            EndpointSet::default(),
        ));

        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:8000/health".to_string())
            .body(Body::empty())?;

        let resp = handle_request::<GraphQLRequest>(req, app_ctx).await?;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(resp.into_body()).await?;
        assert_eq!(body, r#"{"message": "ready"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_graphql_endpoint() -> anyhow::Result<()> {
        let sdl = tokio::fs::read_to_string(tailcall_fixtures::configs::JSONPLACEHOLDER).await?;
        let config = Config::from_sdl(&sdl).to_result()?;
        let mut blueprint = Blueprint::try_from(&ConfigModule::from(config))?;
        blueprint.server.routes = Routes::default().with_graphql("/gql");
        let app_ctx = Arc::new(AppContext::new(
            blueprint,
            init(None),
            EndpointSet::default(),
        ));

        let query = r#"{"query": "{ __schema { queryType { name } } }"}"#;
        let req = Request::builder()
            .method(Method::POST)
            .uri("http://localhost:8000/gql".to_string())
            .header("Content-Type", "application/json")
            .body(Body::from(query))?;

        let resp = handle_request::<GraphQLRequest>(req, app_ctx).await?;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(resp.into_body()).await?;
        let body_str = String::from_utf8(body.to_vec())?;
        assert!(body_str.contains("queryType"));
        assert!(body_str.contains("name"));

        Ok(())
    }

    #[test]
    fn test_create_allowed_headers() {
        use std::collections::BTreeSet;

        use http::header::{HeaderMap, HeaderValue};

        use super::create_allowed_headers;

        let mut headers = HeaderMap::new();
        headers.insert("X-foo", HeaderValue::from_static("bar"));
        headers.insert("x-bar", HeaderValue::from_static("foo"));
        headers.insert("x-baz", HeaderValue::from_static("baz"));

        let allowed = BTreeSet::from_iter(vec!["x-foo".to_string(), "X-bar".to_string()]);

        let new_headers = create_allowed_headers(&headers, &allowed);
        assert_eq!(new_headers.len(), 2);
        assert_eq!(new_headers.get("x-foo").unwrap(), "bar");
        assert_eq!(new_headers.get("x-bar").unwrap(), "foo");
    }
}
