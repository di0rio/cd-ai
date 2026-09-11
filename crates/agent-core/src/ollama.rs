use std::time::Duration;

use serde::Serialize;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub size_bytes: u64,
    pub parameter_size: String,
    pub quantization: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadedModel {
    pub name: String,
    pub size_bytes: u64,
    pub vram_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaStatus {
    pub reachable: bool,
    pub version: Option<String>,
    pub models: Vec<ModelInfo>,
    pub loaded: Vec<LoadedModel>,
    pub error: Option<String>,
}

pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, serde::Deserialize)]
struct VersionResponse {
    version: String,
}

#[derive(Debug, serde::Deserialize)]
struct TagsResponse {
    models: Vec<Tag>,
}

#[derive(Debug, serde::Deserialize)]
struct Tag {
    name: String,
    size: u64,
    #[serde(default)]
    details: TagDetails,
}

#[derive(Debug, serde::Deserialize, Default)]
struct TagDetails {
    #[serde(default)]
    parameter_size: String,
    #[serde(default)]
    quantization_level: String,
}

#[derive(Debug, serde::Deserialize)]
struct PsResponse {
    models: Vec<Ps>,
}

#[derive(Debug, serde::Deserialize)]
struct Ps {
    name: String,
    size: u64,
    size_vram: u64,
}

impl From<Tag> for ModelInfo {
    fn from(tag: Tag) -> Self {
        Self {
            name: tag.name,
            size_bytes: tag.size,
            parameter_size: tag.details.parameter_size,
            quantization: tag.details.quantization_level,
        }
    }
}

impl From<Ps> for LoadedModel {
    fn from(ps: Ps) -> Self {
        Self {
            name: ps.name,
            size_bytes: ps.size,
            vram_bytes: ps.size_vram,
        }
    }
}

impl OllamaClient {
    /// Refuses non-loopback hosts: the app never sends prompts off the machine (decision 0004).
    pub fn new(base_url: &str) -> Result<Self, String> {
        let url =
            reqwest::Url::parse(base_url).map_err(|_| "URL do Ollama inválida".to_string())?;
        let loopback = matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1")
        );
        if url.scheme() != "http" || !loopback {
            return Err("o Ollama precisa estar nesta máquina (loopback)".to_string());
        }
        let base_url = base_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { base_url, http })
    }

    /// Never fails; unreachable => reachable: false + error.
    pub async fn status(&self) -> OllamaStatus {
        let version_url = format!("{}/api/version", self.base_url);
        let version = match self.http.get(&version_url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<VersionResponse>().await {
                    Ok(parsed) => Some(parsed.version),
                    Err(error) => {
                        return OllamaStatus {
                            reachable: true,
                            version: None,
                            models: vec![],
                            loaded: vec![],
                            error: Some(error.to_string()),
                        };
                    }
                }
            }
            Ok(response) => {
                return OllamaStatus {
                    reachable: true,
                    version: None,
                    models: vec![],
                    loaded: vec![],
                    error: Some(format!("Ollama respondeu com status {}", response.status())),
                };
            }
            Err(error) => {
                return OllamaStatus {
                    reachable: false,
                    version: None,
                    models: vec![],
                    loaded: vec![],
                    error: Some(error.to_string()),
                };
            }
        };

        let mut error = None;
        let models = {
            let url = format!("{}/api/tags", self.base_url);
            match self.http.get(&url).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.json::<TagsResponse>().await {
                        Ok(parsed) => parsed.models.into_iter().map(ModelInfo::from).collect(),
                        Err(cause) => {
                            error = Some(cause.to_string());
                            vec![]
                        }
                    }
                }
                Ok(response) => {
                    error = Some(format!("Ollama respondeu com status {}", response.status()));
                    vec![]
                }
                Err(cause) => {
                    error = Some(cause.to_string());
                    vec![]
                }
            }
        };
        let loaded = {
            let url = format!("{}/api/ps", self.base_url);
            match self.http.get(&url).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.json::<PsResponse>().await {
                        Ok(parsed) => parsed.models.into_iter().map(LoadedModel::from).collect(),
                        Err(cause) => {
                            error = Some(cause.to_string());
                            vec![]
                        }
                    }
                }
                Ok(response) => {
                    error = Some(format!("Ollama respondeu com status {}", response.status()));
                    vec![]
                }
                Err(cause) => {
                    error = Some(cause.to_string());
                    vec![]
                }
            }
        };

        OllamaStatus {
            reachable: true,
            version,
            models,
            loaded,
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_models(json: &str) -> Result<Vec<ModelInfo>, serde_json::Error> {
        let response: TagsResponse = serde_json::from_str(json)?;
        Ok(response.models.into_iter().map(ModelInfo::from).collect())
    }

    fn parse_loaded(json: &str) -> Result<Vec<LoadedModel>, serde_json::Error> {
        let response: PsResponse = serde_json::from_str(json)?;
        Ok(response.models.into_iter().map(LoadedModel::from).collect())
    }

    #[test]
    fn new_accepts_loopback() {
        assert!(OllamaClient::new("http://127.0.0.1:11434").is_ok());
        assert!(OllamaClient::new("http://localhost:11434/").is_ok());
    }

    #[test]
    fn new_refuses_remote_host() {
        assert!(OllamaClient::new("http://192.168.0.10:11434").is_err());
        assert!(OllamaClient::new("https://example.com").is_err());
    }

    #[test]
    fn parses_tags() {
        let json = r#"{"models":[{"name":"qwen3:4b","size":2500000000,"details":{"parameter_size":"4.0B","quantization_level":"Q4_K_M","family":"qwen3"}}]}"#;
        let models = parse_models(json).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "qwen3:4b");
        assert_eq!(models[0].quantization, "Q4_K_M");
    }

    #[test]
    fn parses_tags_without_details() {
        let json = r#"{"models":[{"name":"x","size":1}]}"#;
        let models = parse_models(json).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].parameter_size, "");
    }

    #[test]
    fn parses_ps() {
        let json = r#"{"models":[{"name":"qwen3:4b","size":3900000000,"size_vram":3900000000}]}"#;
        let loaded = parse_loaded(json).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].vram_bytes, 3900000000);
    }

    #[tokio::test]
    #[ignore]
    async fn live_status() {
        let client = OllamaClient::new(DEFAULT_BASE_URL).unwrap();
        assert!(client.status().await.reachable);
    }
}
