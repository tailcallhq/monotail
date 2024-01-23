use std::pin::Pin;
use std::sync::Arc;

use futures_util::future::join_all;
use futures_util::Future;

use crate::http::Response;
use crate::{EventHandler, HttpIO};

#[derive(Clone)]
pub struct HttpHook {
  client: Arc<dyn HttpIO + Send + Sync>,
  handler: Arc<dyn EventHandler<Event, Command> + Send + Sync>,
}

pub enum Event {
  Request(reqwest::Request),
  Response(Vec<Response<hyper::body::Bytes>>),
}

pub enum Command {
  Request(Vec<reqwest::Request>),
  Response(Response<hyper::body::Bytes>),
}

impl HttpHook {
  pub fn new(
    http: Arc<dyn HttpIO + Send + Sync>,
    handler: Arc<dyn EventHandler<Event, Command> + Send + Sync>,
  ) -> Self {
    HttpHook { client: http, handler }
  }

  fn on_command<'a>(
    &'a self,
    command: Command,
  ) -> Pin<Box<dyn Future<Output = anyhow::Result<Response<hyper::body::Bytes>>> + Send + 'a>> {
    Box::pin(async move {
      match command {
        Command::Request(requests) => {
          let responses = join_all(requests.into_iter().map(|request| self.client.execute(request)))
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()?;

          let command = self.handler.on_event(Event::Response(responses))?;
          Ok(self.on_command(command).await?)
        }
        Command::Response(response) => {
          let command = self.handler.on_event(Event::Response(vec![response]))?;
          Ok(self.on_command(command).await?)
        }
      }
    })
  }
}

#[async_trait::async_trait]
impl HttpIO for HttpHook {
  async fn execute(&self, request: reqwest::Request) -> anyhow::Result<Response<hyper::body::Bytes>> {
    let command = self.handler.on_event(Event::Request(request))?;
    self.on_command(command).await
  }
}
