use std::time::Duration;
#[cfg(feature = "default")]
use http_cache_reqwest::{Cache, CacheMode, HttpCache, HttpCacheOptions, MokaManager};
use reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};

use super::Response;
use crate::config::Upstream;

#[async_trait::async_trait]
pub trait HttpClient: Sync + Send {
  fn execute(&self, req: reqwest::Request) -> anyhow::Result<Response>;
}

#[async_trait::async_trait]
impl HttpClient for DefaultHttpClient {
  fn execute(&self, req: reqwest::Request) -> anyhow::Result<Response> {
    // todo!();
    /*let x = async_std::task::block_on(async move {
      let response = self.execute(req).await;
      return match response {
        Ok(resource) => Ok(resource),
        Err(e) => {
          Err(anyhow::anyhow!("{}",e.to_string()))
        }
      }
    });*/
    Ok(Response::default())
  }
}

#[derive(Clone)]
pub struct DefaultHttpClient {
  client: ClientWithMiddleware,
}

impl Default for DefaultHttpClient {
  fn default() -> Self {
    let upstream = Upstream::default();
    //TODO: default is used only in tests. Drop default and move it to test.
    DefaultHttpClient::new(&upstream)
  }
}

impl DefaultHttpClient {
  pub fn new(upstream: &Upstream) -> Self {
    let mut builder = Client::builder();
      // .tcp_keepalive(Some(Duration::from_secs(upstream.get_tcp_keep_alive())))
      // .timeout(Duration::from_secs(upstream.get_timeout()))
      // .connect_timeout(Duration::from_secs(upstream.get_connect_timeout()))
      // .http2_keep_alive_interval(Some(Duration::from_secs(upstream.get_keep_alive_interval())))
      // .http2_keep_alive_timeout(Duration::from_secs(upstream.get_keep_alive_timeout()))
      // .http2_keep_alive_while_idle(upstream.get_keep_alive_while_idle())
      // .pool_idle_timeout(Some(Duration::from_secs(upstream.get_pool_idle_timeout())))
      // .pool_max_idle_per_host(upstream.get_pool_max_idle_per_host())
      // .user_agent(upstream.get_user_agent());
    #[cfg(feature = "default")]
    if let Some(ref proxy) = upstream.proxy {
      builder = builder.proxy(reqwest::Proxy::http(proxy.url.clone()).expect("Failed to set proxy in http client"));
    }

    let mut client = ClientBuilder::new(builder.build().expect("Failed to build client"));
    #[cfg(feature = "default")]
    if upstream.get_enable_http_cache() {
      client = client.with(Cache(HttpCache {
        mode: CacheMode::Default,
        manager: MokaManager::default(),
        options: HttpCacheOptions::default(),
      }))
    }

    DefaultHttpClient { client: client.build() }
  }

  pub async fn execute(&self, request: reqwest::Request) -> reqwest_middleware::Result<Response> {
    log::info!("{} {} ", request.method(), request.url());
    let response = self.client.execute(request).await?.error_for_status()?;
    let response = Response::from_response(response).await?;
    Ok(response)
  }
}
