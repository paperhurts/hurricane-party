//! The one door to the internet (D19, D29).
//!
//! The app plays local files and opens no connection of its own at playback
//! time (D11). yt-dlp and ffmpeg download what a person asks for, as programs
//! of their own. The one thing this process fetches is the Cone theme's
//! radar and alerts (#85), on a timer, and every such request goes through
//! [`get`], to a host in [`ALLOWED`]. Nothing else in the crate builds an HTTP
//! client: this function is what a search for one should find.

use std::time::Duration;

/// Where a request may go: NOAA's radar image service and the National
/// Weather Service's API.
pub const ALLOWED: [&str; 2] = ["mapservices.weather.noaa.gov", "api.weather.gov"];

/// `api.weather.gov` refuses a request without a User-Agent (403), and asks
/// for one that names the application and where to reach whoever runs it.
/// The repository is that place; no address of anyone's is in it.
pub const USER_AGENT: &str = "hurricane-party (github.com/paperhurts/hurricane-party)";

/// Whether a URL may be fetched: https, to an allowed host, nothing else.
pub fn allowed(raw: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(raw).map_err(|e| format!("not a URL: {e}"))?;
    if u.scheme() != "https" {
        return Err(format!("{} is not https", u.scheme()));
    }
    match u.host_str() {
        Some(h) if ALLOWED.contains(&h) => Ok(u),
        Some(h) => Err(format!("{h} is not a host this app talks to")),
        None => Err("no host".into()),
    }
}

/// GET a URL on an allowed host and return its body. A slow or dead
/// connection gives up after half a minute rather than hanging a refresh.
pub async fn get(raw: &str) -> Result<Vec<u8>, String> {
    let u = allowed(raw)?;
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("no HTTP client: {e}"))?;
    let res = client
        .get(u)
        .header(
            "Accept",
            "application/geo+json, application/json, image/png",
        )
        .send()
        .await
        .map_err(|e| format!("could not reach the weather service: {e}"))?
        .error_for_status()
        .map_err(|e| format!("the weather service refused: {e}"))?;
    res.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("the weather service's answer broke off: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_to_the_weather_services_is_allowed() {
        assert!(allowed("https://api.weather.gov/alerts/active?area=FL").is_ok());
        assert!(allowed("https://mapservices.weather.noaa.gov/eventdriven/rest").is_ok());
        assert!(allowed("http://api.weather.gov/alerts").is_err());
        assert!(allowed("https://example.com/").is_err());
        // A lookalike host is not the host.
        assert!(allowed("https://api.weather.gov.example.com/").is_err());
        assert!(allowed("https://user@evil.example/?api.weather.gov").is_err());
        assert!(allowed("file:///C:/Windows").is_err());
        assert!(allowed("not a url").is_err());
    }
}
