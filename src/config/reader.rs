use anyhow::anyhow;
#[cfg(feature = "default")]
use tokio::{fs::File, io::AsyncReadExt};
use url::Url;

use crate::config::{Config, Source};
pub struct ConfigReader {
  file_paths: Vec<String>,
}

impl ConfigReader {
  pub fn init<Iter>(file_paths: Iter) -> Self
  where
    Iter: Iterator,
    Iter::Item: AsRef<str>,
  {
    Self { file_paths: file_paths.map(|path| path.as_ref().to_owned()).collect() }
  }
  pub async fn read(&self) -> anyhow::Result<Config> {
    let mut config = Config::default();
    #[cfg(feature = "default")]
    // we don't need this function for worker
    // but it's called elsewhere and we are sure that this won't be called from worker
    // so for sake of readability we put the parts of function under feature instead of the function
    for path in &self.file_paths {
      let conf = if let Ok(url) = Url::parse(path) {
        Self::from_url(url).await?
      } else {
        let path = path.trim_end_matches('/');
        Self::from_file_path(path).await?
      };
      config = config.clone().merge_right(&conf);
    }
    Ok(config)
  }
  #[cfg(feature = "default")]
  async fn from_file_path(file_path: &str) -> anyhow::Result<Config> {
    let (server_sdl, source) = ConfigReader::read_file(file_path).await?;
    Config::from_source(source, &server_sdl)
  }
  #[cfg(feature = "default")]
  async fn read_file(file_path: &str) -> anyhow::Result<(String, Source)> {
    let mut f = File::open(file_path).await?;
    let mut buffer = Vec::new();
    f.read_to_end(&mut buffer).await?;
    Ok((String::from_utf8(buffer)?, Source::detect(file_path)?))
  }
  async fn read_over_url(url: Url) -> anyhow::Result<(String, Source)> {
    let path = url.path().to_string();
    let resp = reqwest::get(url).await?;
    if !resp.status().is_success() {
      return Err(anyhow!("Read over URL failed with status code: {}", resp.status()));
    }
    let source = if let Some(v) = resp.headers().get("content-type") {
      if let Ok(s) = Source::detect_content_type(v.to_str()?) {
        s
      } else {
        Source::detect(path.trim_end_matches('/'))?
      }
    } else {
      Source::detect(path.trim_end_matches('/'))?
    };
    let txt = resp.text().await?;
    Ok((txt, source))
  }
  async fn from_url(url: Url) -> anyhow::Result<Config> {
    let (st, source) = Self::read_over_url(url).await?;
    Config::from_source(source, &st)
  }
}
#[cfg(test)]
mod reader_tests {
  use tokio::io::AsyncReadExt;

  use crate::config::reader::ConfigReader;
  use crate::config::{Config, Type};

  fn start_mock_server(port: u16) -> mockito::Server {
    mockito::Server::new_with_port(port)
  }
  #[tokio::test]
  async fn test_all() {
    let mut cfg = Config::default();
    cfg.schema.query = Some("Test".to_string());
    cfg = cfg.types([("Test", Type::default())].to_vec());

    let mut server = start_mock_server(3080);
    let header_serv = server
      .mock("GET", "/")
      .with_status(200)
      .with_header("content-type", "application/graphql")
      .with_body(cfg.to_sdl())
      .create();
    let mut json = String::new();
    tokio::fs::File::open("examples/jsonplaceholder.json")
      .await
      .unwrap()
      .read_to_string(&mut json)
      .await
      .unwrap();
    let foo_json_serv = server
      .mock("GET", "/foo.json")
      .with_status(200)
      .with_body(json)
      .create();

    let files: Vec<String> = [
      "examples/jsonplaceholder.yml",   // config from local file
      "http://localhost:3080/",         // with content-type header
      "http://localhost:3080/foo.json", // with url extension
    ]
    .iter()
    .map(|x| x.to_string())
    .collect();
    let cr = ConfigReader::init(files.iter());
    let c = cr.read().await.unwrap();
    assert_eq!(
      ["Post", "Query", "Test", "User"]
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<String>>(),
      c.types.keys().map(|i| i.to_string()).collect::<Vec<String>>()
    );
    foo_json_serv.assert(); // checks if the request was actually made
    header_serv.assert();
  }
  #[tokio::test]
  async fn test_local_files() {
    let files: Vec<String> = [
      "examples/jsonplaceholder.yml",
      "examples/jsonplaceholder.graphql",
      "examples/jsonplaceholder.json",
    ]
    .iter()
    .map(|x| x.to_string())
    .collect();
    let cr = ConfigReader::init(files.iter());
    let c = cr.read().await.unwrap();
    assert_eq!(
      ["Post", "Query", "User"]
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<String>>(),
      c.types.keys().map(|i| i.to_string()).collect::<Vec<String>>()
    );
  }
}
