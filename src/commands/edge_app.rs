use crate::authentication::Authentication;
use crate::commands::CommandError;
use std::collections::HashMap;

use log::debug;
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeAppManifest {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub icon: String,
    pub author: String,
    pub homepage_url: String,
}

pub struct EdgeAppCommand {
    authentication: Authentication,
}

impl EdgeAppCommand {
    pub fn new(authentication: Authentication) -> Self {
        Self { authentication }
    }

    pub fn init(self, path: &Path) -> Result<(), CommandError> {
        let mut object = serde_yaml::to_value(EdgeAppManifest::default())?;
        let map = object.as_mapping_mut().ok_or(CommandError::MissingField)?;

        // following fields will be generated server side when publishing.
        map.remove("id");

        let yaml = serde_yaml::to_string(map)?;
        let input = File::create(path)?;
        write!(&input, "{yaml}")?;

        Ok(())
    }

    pub fn publish(self, path: &Path) -> Result<EdgeAppManifest, CommandError> {
        let url = format!(
            "{}/v4/edge_apps?select=id,name,version,description,icon,author,homepage_url",
            &self.authentication.config.url
        );

        let data = fs::read_to_string(path)?;

        // by converting to struct we make sure there no extra fields and all required fields are present
        let _: EdgeAppManifest = serde_yaml::from_str(&data)?;
        let mut payload: HashMap<String, String> = serde_yaml::from_str(&data)?;

        // Id can not be empty when posting. Depending on what we decide I should either raise an error
        // or we allow users to supply an id.
        if payload.contains_key("id") && !payload["id"].is_empty() {
            return Err(CommandError::InvalidManifestValue(
                "Only empty id accepted when publishing manifest".to_owned(),
            ));
        }

        payload.remove("id");

        // for now all values are fields in the manifest are required to be non-empty.
        // if we change that we will need to have a list of required non-empty values.
        debug!("Edge app headers: ");
        for (k, v) in &payload {
            debug!("{k}: {v}");
            if v.is_empty() {
                return Err(CommandError::InvalidManifestValue(k.to_string()));
            };
        }

        let mut headers = HeaderMap::new();
        headers.insert("Prefer", "return=representation".parse()?);

        let response = self
            .authentication
            .build_client()?
            .post(url)
            .headers(headers)
            .json(&payload)
            .send()?;

        if response.status() != StatusCode::CREATED {
            return Err(CommandError::WrongResponseStatus(
                response.status().as_u16(),
            ));
        }

        // I think there is no need to check for size - this vector should always be size 1.
        // We can let it crash in run time otherwise.
        let updated_manifests: Vec<EdgeAppManifest> = serde_json::from_str(&response.text()?)?;

        // overwrite manifest file now that we have id and extra fields.
        let yaml = serde_yaml::to_string(&updated_manifests[0])?;
        let input = File::create(path)?;
        write!(&input, "{yaml}")?;

        Ok(updated_manifests[0].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentication::Config;

    use envtestkit::lock::lock_test;
    use envtestkit::set_env;
    use httpmock::Method::POST;
    use httpmock::MockServer;
    use std::ffi::OsString;
    use tempdir::TempDir;

    #[test]
    fn test_edge_app_init_should_create_screenly_yml() {
        let tmp_dir = TempDir::new("test").unwrap();
        let _lock = lock_test();
        let _test = set_env(OsString::from("HOME"), tmp_dir.path().to_str().unwrap());
        fs::write(tmp_dir.path().join(".screenly").to_str().unwrap(), "token").unwrap();

        // init should make no requests so it's whatever for server url
        let config = Config::new("asdf".to_owned());
        let authentication = Authentication::new_with_config(config);
        let command = EdgeAppCommand::new(authentication);

        let p = tmp_dir.path().join("screenly.yml");
        assert!(command.init(Path::new(p.to_str().unwrap())).is_ok());

        let expected = r#"homepage_url: ''
name: ''
author: ''
icon: ''
version: ''
description: ''
"#;

        assert_eq!(
            serde_yaml::from_str::<EdgeAppManifest>(expected).unwrap(),
            serde_yaml::from_str::<EdgeAppManifest>(
                &fs::read_to_string(Path::new(p.to_str().unwrap())).unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn test_edge_app_publish_should_send_correct_request() {
        let tmp_dir = TempDir::new("test").unwrap();
        let _lock = lock_test();
        let _test = set_env(OsString::from("HOME"), tmp_dir.path().to_str().unwrap());
        fs::write(tmp_dir.path().join(".screenly").to_str().unwrap(), "token").unwrap();
        let manifest = EdgeAppManifest {
            id: "".to_string(),
            name: "Test".to_string(),
            version: "100".to_string(),
            description: "Best".to_string(),
            icon: "?".to_string(),
            author: "Best author ever".to_string(),
            homepage_url: "test.io".to_string(),
        };

        let published_manifest = vec![EdgeAppManifest {
            id: "01GS5H2CX6Y10ZRJHEDQPEWN4E".to_string(),
            ..manifest.clone()
        }];

        let mut binding = serde_json::to_value(&manifest).unwrap();
        let manifest_object = binding.as_object_mut().unwrap();
        manifest_object.remove("id");

        let mock_server = MockServer::start();
        mock_server.mock(|when, then| {
            when.method(POST)
                .path("/v4/edge_apps")
                .header("Authorization", "Token token")
                .header(
                    "user-agent",
                    format!("screenly-cli {}", env!("CARGO_PKG_VERSION")),
                )
                .json_body(serde_json::to_value(manifest_object).unwrap());

            then.status(201)
                .json_body(serde_json::to_value(&published_manifest).unwrap());
        });

        let config = Config::new(mock_server.base_url());
        let authentication = Authentication::new_with_config(config);
        let command = EdgeAppCommand::new(authentication);

        let path = tmp_dir.path().join("screenly.yml");

        let mut object = serde_yaml::to_value(&manifest).unwrap();
        let map = object.as_mapping_mut().unwrap();

        // following fields will be generated server side when publishing.
        map.remove("id");
        map.remove("created_at");
        map.remove("created_by");
        map.remove("updated_at");
        map.remove("permissions");

        let yaml = serde_yaml::to_string(map).unwrap();
        let input = File::create(Path::new(path.to_str().unwrap())).unwrap();
        write!(&input, "{yaml}").unwrap();

        let manifest_from_server = command.publish(Path::new(path.to_str().unwrap())).unwrap();
        assert_eq!(manifest_from_server, published_manifest[0]);

        // also check that file was updated
        let manifest_from_file = serde_yaml::from_str::<EdgeAppManifest>(
            &fs::read_to_string(Path::new(path.to_str().unwrap())).unwrap(),
        )
        .unwrap();

        assert_eq!(manifest_from_file, manifest_from_server);
    }
}
