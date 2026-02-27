use super::error::ToolError;
use crate::config::Config;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct WebSearchArgs {
    pub query: String,
    #[serde(default = "default_count")]
    pub count: i64,
}

fn default_count() -> i64 {
    5
}

#[derive(Clone)]
pub struct WebSearch {
    pub config: Arc<Config>,
    pub client: reqwest::Client,
}

impl Tool for WebSearch {
    const NAME: &'static str = "web_search";

    type Error = ToolError;
    type Args = WebSearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Perform web search".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match self.config.search_provider.as_str() {
            "serper" => self.search_serper(args).await,
            _ => self.search_brave(args).await,
        }
    }
}

impl WebSearch {
    async fn search_brave(&self, args: WebSearchArgs) -> Result<String, ToolError> {
        let api_key = self
            .config
            .search_api_key
            .as_ref()
            .ok_or_else(|| ToolError::SearchFailed("Search API key not set".to_string()))?;

        let response = self
            .client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", api_key)
            .header("Accept", "application/json")
            .query(&[
                ("q", args.query.as_str()),
                ("count", args.count.to_string().as_str()),
            ])
            .send()
            .await
            .map_err(|e| ToolError::SearchFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::SearchFailed(format!(
                "Brave API HTTP {}",
                response.status()
            )));
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ToolError::SearchFailed(e.to_string()))?;

        Ok(serde_json::to_string_pretty(&data).unwrap_or_default())
    }

    async fn search_serper(&self, args: WebSearchArgs) -> Result<String, ToolError> {
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
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::SearchFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::SearchFailed(format!(
                "Serper API HTTP {}",
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
