use std::{env, fs};

use reqwest::header::{HeaderMap, InvalidHeaderValue};
use reqwest::{header, StatusCode};
use thiserror::Error;

const API_BASE_URL: &str = "https://api.screenlyapp.com/api";

pub struct Config {
    pub url: String,
}

#[derive(Error, Debug)]
pub enum AuthenticationError {
    #[error("wrong credentials error")]
    WrongCredentials,
    #[error("no credentials error")]
    NoCredentials,
    #[error("request error")]
    Request(#[from] reqwest::Error),
    #[error("i/o error")]
    Io(#[from] std::io::Error),
    #[error("env error")]
    Env(#[from] env::VarError),
    #[error("missing home dir error")]
    MissingHomeDir(),
    #[error("invalid header error")]
    InvalidHeader(#[from] InvalidHeaderValue),
    #[error("unknown error")]
    Unknown,
}

pub struct Authentication {
    pub config: Config,
}

impl Config {
    pub fn default() -> Self {
        Self {
            url: API_BASE_URL.to_string(),
        }
    }

    #[cfg(test)]
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

impl Authentication {
    pub fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }

    pub fn read_token() -> Result<String, AuthenticationError> {
        if let Ok(token) = env::var("API_TOKEN") {
            return Ok(token);
        }

        match dirs::home_dir() {
            Some(path) => {
                fs::read_to_string(path.join(".screenly")).map_err(AuthenticationError::Io)
            }
            None => Err(AuthenticationError::NoCredentials),
        }
    }

    #[cfg(test)]
    pub fn new_with_config(config: Config) -> Self {
        Self { config }
    }

    pub fn verify_and_store_token(&self, token: &str) -> anyhow::Result<(), AuthenticationError> {
        self.verify_token(token)?;

        match dirs::home_dir() {
            Some(home) => {
                fs::write(home.join(".screenly"), token)?;
                Ok(())
            }
            None => Err(AuthenticationError::MissingHomeDir()),
        }
    }

    fn verify_token(&self, token: &str) -> anyhow::Result<(), AuthenticationError> {
        // Using uuid of non existing playlist. If we get 404 it means we authenticated successfully.
        let url = format!("{}/v3/groups/11CF9Z3GZR0005XXKH00F8V20R/", &self.config.url);
        let secret = format!("Token {token}");
        let client = reqwest::blocking::Client::builder().build()?;

        let res = client
            .get(url)
            .header(header::AUTHORIZATION, &secret)
            .send()?;

        match res.status() {
            StatusCode::UNAUTHORIZED => Err(AuthenticationError::WrongCredentials),
            StatusCode::NOT_FOUND => Ok(()),
            _ => Err(AuthenticationError::Unknown),
        }
    }

    pub fn build_client(&self) -> Result<reqwest::blocking::Client, AuthenticationError> {
        let token = Authentication::read_token()?;
        let secret = format!("Token {token}");
        let mut default_headers = HeaderMap::new();
        default_headers.insert(header::AUTHORIZATION, secret.parse()?);
        default_headers.insert(
            header::USER_AGENT,
            format!("screenly-cli {}", env!("CARGO_PKG_VERSION")).parse()?,
        );

        reqwest::blocking::Client::builder()
            .default_headers(default_headers)
            .build()
            .map_err(AuthenticationError::Request)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use envtestkit::lock::lock_test;
    use envtestkit::set_env;
    use httpmock::{Method::GET, MockServer};
    use simple_logger::SimpleLogger;
    use tempdir::TempDir;

    use super::*;

    #[test]
    fn test_verify_and_store_token_when_token_is_valid() {
        SimpleLogger::new()
            .with_level(log::LevelFilter::Debug)
            .init()
            .unwrap();
        let tmp_dir = TempDir::new("test").unwrap();
        let _lock = lock_test();
        let _test = set_env(OsString::from("HOME"), tmp_dir.path().to_str().unwrap());
        let mock_server = MockServer::start();
        mock_server.mock(|when, then| {
            when.method(GET)
                .path("/v3/groups/11CF9Z3GZR0005XXKH00F8V20R/")
                .header("Authorization", "Token token");
            then.status(404);
        });

        let config = Config::new(mock_server.base_url());
        let authentication = Authentication::new_with_config(config);
        assert!(authentication
            .verify_and_store_token("correct_token")
            .is_ok());
        let path = tmp_dir.path().join(".screenly");
        assert!(path.exists());
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.eq("correct_token"));
        tmp_dir.close().unwrap();
    }

    #[test]
    fn test_verify_and_store_token_when_token_is_invalid() {
        let tmp_dir = TempDir::new("invalid").unwrap();
        let _lock = lock_test();
        let _test = set_env(OsString::from("HOME"), tmp_dir.path().to_str().unwrap());
        let mock_server = MockServer::start();
        mock_server.mock(|when, then| {
            when.method(GET)
                .path("/v3/groups/11CF9Z3GZR0005XXKH00F8V20R/");
            then.status(401);
        });

        let config = Config::new(mock_server.base_url());
        let authentication = Authentication::new_with_config(config);
        assert!(authentication
            .verify_and_store_token("wrong_token")
            .is_err());
        let path = tmp_dir.path().join(".screenly");
        assert!(!path.exists());
    }

    #[test]
    fn test_read_token_when_token_is_overridden_with_env_variable_correct_token_is_returned() {
        let tmp_dir = TempDir::new("test").unwrap();
        let _lock = lock_test();
        let _token = set_env(OsString::from("API_TOKEN"), "env_token");
        let _test = set_env(OsString::from("HOME"), tmp_dir.path().to_str().unwrap());
        println!("{}", tmp_dir.path().join(".screenly").to_str().unwrap());
        fs::write(tmp_dir.path().join(".screenly").to_str().unwrap(), "token").unwrap();
        assert_eq!(Authentication::read_token().unwrap(), "env_token");
    }

    #[test]
    fn test_read_token_correct_token_is_returned() {
        let tmp_dir = TempDir::new("test").unwrap();
        let _lock = lock_test();
        let _test = set_env(OsString::from("HOME"), tmp_dir.path().to_str().unwrap());
        fs::write(tmp_dir.path().join(".screenly").to_str().unwrap(), "token").unwrap();

        assert_eq!(Authentication::read_token().unwrap(), "token");
    }
}
