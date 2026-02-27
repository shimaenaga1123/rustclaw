use super::error::ToolError;
use crate::config::Config;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_FETCH_TOKEN: usize = 50000;

#[derive(Deserialize, Serialize)]
pub struct WebFetchArgs {
    pub url: String,
}

#[derive(Clone)]
pub struct WebFetch {
    pub config: Config,
    pub client: reqwest::Client,
}

impl Tool for WebFetch {
    const NAME: &'static str = "web_fetch";

    type Error = ToolError;
    type Args = WebFetchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Perform web fetch with LLM-friendly format".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch"
                    },
                },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Smart-fetch with jina.ai
        self.fetch_jina_free(args).await
    }
}

impl WebFetch {
    // with API Key for jina.ai
    async fn fetch_jina(&self, args: WebFetchArgs) -> Result<String, ToolError> {
        let api_key = self
            .config
            .fetch
            .api_key
            .as_ref()
            .ok_or_else(|| ToolError::FetchFailed("Fetch API key not set".to_string()))?;

        let url = format!("https://r.jina.ai/{}", args.url);
        let authorization = format!("Bearer {}", api_key);
        let response = self
            .client
            .get(url)
            .header("Authorization", authorization)
            .header("X-Engine", "browser")
            .header("X-Token-Budget", MAX_FETCH_TOKEN.to_string())
            .send()
            .await
            .map_err(|e| ToolError::FetchFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::FetchFailed(format!(
                "JINA API HTTP {}",
                response.status()
            )));
        }

        let text = response
            .text()
            .await
            .map_err(|e| ToolError::FetchFailed(e.to_string()))?;

        Ok(text)
    }
    async fn fetch_jina_free(&self, args: WebFetchArgs) -> Result<String, ToolError> {
        let url = format!("https://r.jina.ai/{}", args.url);
        let response = self
            .client
            .get(url)
            .header("X-Engine", "browser")
            .header("X-Token-Budget", MAX_FETCH_TOKEN.to_string())
            .send()
            .await
            .map_err(|e| ToolError::FetchFailed(e.to_string()))?;

        if !response.status().is_success() {
            return if self.config.fetch.api_key.is_some() {
                self.fetch_jina(args).await
            } else {
                Err(ToolError::FetchFailed(format!(
                    "JINA API HTTP {}",
                    response.status()
                )))
            };
        }

        let text = response
            .text()
            .await
            .map_err(|e| ToolError::FetchFailed(e.to_string()))?;

        Ok(text)
    }
}
