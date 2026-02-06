use super::error::ToolError;
use crate::config::Config;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

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
    pub config: Config,
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
        let api_key = self
            .config
            .brave_api_key
            .as_ref()
            .ok_or_else(|| ToolError::SearchFailed("API key not set".to_string()))?;

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
                "HTTP {}",
                response.status()
            )));
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ToolError::SearchFailed(e.to_string()))?;

        let results = data
            .get("web")
            .and_then(|v| v.get("results"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::SearchFailed("Invalid response".to_string()))?;

        if results.is_empty() {
            return Ok("No results".to_string());
        }

        let mut output = format!("Search results ({} items):\n\n", results.len());

        for (i, result) in results.iter().enumerate() {
            let title = result
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("No title");
            let url = result.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let description = result
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            output.push_str(&format!(
                "{}. {}\n   {}\n   {}\n\n",
                i + 1,
                title,
                url,
                description
            ));
        }

        Ok(output)
    }
}
