use super::error::ToolError;
use crate::config::Config;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, Serialize)]
pub struct WebNewsArgs {
    pub query: String,
}

#[derive(Clone)]
pub struct WebNews {
    pub config: Config,
    pub client: reqwest::Client,
}

impl Tool for WebNews {
    const NAME: &'static str = "web_news";

    type Error = ToolError;
    type Args = WebNewsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search for recent news articles".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "News search query"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let api_key = self
            .config
            .search_api_key
            .as_ref()
            .ok_or_else(|| ToolError::SearchFailed("Search API key not set".to_string()))?;

        let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string());
        let parts: Vec<&str> = locale.split(['-', '_']).collect();

        let hl = parts.first().unwrap_or(&"en").to_lowercase();
        let gl = parts.last().unwrap_or(&"us").to_lowercase();

        let body = json!({
            "q": args.query,
            "gl": gl,
            "hl": hl,
        });

        let response = self
            .client
            .post("https://google.serper.dev/news")
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::SearchFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::SearchFailed(format!(
                "Serper News API HTTP {}",
                response.status()
            )));
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ToolError::SearchFailed(e.to_string()))?;

        Ok(serde_json::to_string_pretty(&data).unwrap_or_default())
    }
}
