use super::error::ToolError;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, Serialize)]
pub struct WeatherArgs {
    pub location: String,
    #[serde(default = "default_forecast_days")]
    pub forecast_days: i64,
}

fn default_forecast_days() -> i64 {
    1
}

#[derive(Clone)]
pub struct Weather {
    pub client: reqwest::Client,
}

#[derive(Deserialize)]
struct GeocodingResponse {
    results: Option<Vec<GeocodingResult>>,
}

#[derive(Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
    admin1: Option<String>,
}

#[derive(Deserialize)]
struct WeatherResponse {
    current: Option<CurrentWeather>,
    daily: Option<DailyWeather>,
    daily_units: Option<DailyUnits>,
}

#[derive(Deserialize)]
struct CurrentWeather {
    temperature_2m: Option<f64>,
    relative_humidity_2m: Option<i64>,
    apparent_temperature: Option<f64>,
    weather_code: Option<i64>,
    wind_speed_10m: Option<f64>,
}

#[derive(Deserialize)]
struct DailyWeather {
    time: Vec<String>,
    weather_code: Vec<i64>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    precipitation_probability_max: Option<Vec<i64>>,
}

#[derive(Deserialize)]
struct DailyUnits {
    temperature_2m_max: String,
    temperature_2m_min: String,
}

impl Tool for Weather {
    const NAME: &'static str = "weather";

    type Error = ToolError;
    type Args = WeatherArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get current weather and forecast for a location by name".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "Location name to search (city, region, or country)"
                    },
                    "forecast_days": {
                        "type": "integer",
                        "description": "Number of forecast days (1-7)",
                        "default": 1
                    }
                },
                "required": ["location"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let forecast_days = args.forecast_days.clamp(1, 7);

        let geo_response = self
            .client
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[
                ("name", args.location.as_str()),
                ("count", "1"),
                ("language", "en"),
                ("format", "json"),
            ])
            .send()
            .await
            .map_err(|e| ToolError::WeatherFailed(e.to_string()))?;

        if !geo_response.status().is_success() {
            return Err(ToolError::WeatherFailed(format!(
                "Geocoding API error: HTTP {}",
                geo_response.status()
            )));
        }

        let geo_data: GeocodingResponse = geo_response
            .json()
            .await
            .map_err(|e| ToolError::WeatherFailed(format!("Failed to parse geocoding: {}", e)))?;

        let location = geo_data
            .results
            .and_then(|r| r.into_iter().next())
            .ok_or_else(|| {
                ToolError::WeatherFailed(format!("Location '{}' not found", args.location))
            })?;

        let weather_response = self
            .client
            .get("https://api.open-meteo.com/v1/forecast")
            .query(&[
                ("latitude", location.latitude.to_string()),
                ("longitude", location.longitude.to_string()),
                (
                    "current",
                    "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m".to_string(),
                ),
                (
                    "daily",
                    "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max".to_string(),
                ),
                ("timezone", "auto".to_string()),
                ("forecast_days", forecast_days.to_string()),
            ])
            .send()
            .await
            .map_err(|e| ToolError::WeatherFailed(e.to_string()))?;

        if !weather_response.status().is_success() {
            return Err(ToolError::WeatherFailed(format!(
                "Weather API error: HTTP {}",
                weather_response.status()
            )));
        }

        let weather: WeatherResponse = weather_response
            .json()
            .await
            .map_err(|e| ToolError::WeatherFailed(format!("Failed to parse weather: {}", e)))?;

        let location_str = format!(
            "{}, {}",
            location.name,
            location.admin1.or(location.country).unwrap_or_default()
        );

        let mut output = format!("Weather for {}\n", location_str);
        output.push_str(&format!(
            "Coordinates: {:.4}, {:.4}\n\n",
            location.latitude, location.longitude
        ));

        if let Some(current) = weather.current {
            output.push_str("## Current Conditions\n");

            if let Some(temp) = current.temperature_2m {
                output.push_str(&format!("Temperature: {:.1}°C", temp));
                if let Some(feels) = current.apparent_temperature {
                    output.push_str(&format!(" (feels like {:.1}°C)", feels));
                }
                output.push('\n');
            }

            if let Some(humidity) = current.relative_humidity_2m {
                output.push_str(&format!("Humidity: {}%\n", humidity));
            }

            if let Some(wind) = current.wind_speed_10m {
                output.push_str(&format!("Wind: {:.1} km/h\n", wind));
            }

            if let Some(code) = current.weather_code {
                output.push_str(&format!("Conditions: {}\n", weather_code_to_string(code)));
            }
        }

        if let Some(daily) = weather.daily {
            output.push_str("\n## Forecast\n");

            let unit = weather
                .daily_units
                .map(|u| u.temperature_2m_max)
                .unwrap_or_else(|| "°C".to_string());

            for (i, date) in daily.time.iter().enumerate() {
                let max = daily.temperature_2m_max.get(i).unwrap_or(&0.0);
                let min = daily.temperature_2m_min.get(i).unwrap_or(&0.0);
                let code = daily.weather_code.get(i).unwrap_or(&0);
                let precip = daily
                    .precipitation_probability_max
                    .as_ref()
                    .and_then(|p| p.get(i))
                    .copied();

                output.push_str(&format!(
                    "{}: {:.0}/{:.0}{} - {}",
                    date,
                    max,
                    min,
                    unit,
                    weather_code_to_string(*code)
                ));

                if let Some(p) = precip {
                    output.push_str(&format!(" ({}% precip)", p));
                }
                output.push('\n');
            }
        }

        Ok(output)
    }
}

fn weather_code_to_string(code: i64) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Foggy",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow grains",
        80 | 81 | 82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Unknown",
    }
}
