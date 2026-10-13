//! Public OpenRouter catalog lookups for model search and selection checks.
//! These endpoints take no credentials and receive no user text; they describe
//! routes and providers, not the account's own provider filters.
use crate::{
    ai::{self, AiModel},
    store::Store,
};
use serde::Serialize;
use std::{collections::HashSet, sync::atomic::AtomicBool, time::Duration};

const ZDR_URL: &str = "https://openrouter.ai/api/v1/endpoints/zdr";
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENDPOINTS_BYTES: usize = 256 * 1024;
/// The ZDR list is one entry per provider route per model — hundreds of rows,
/// much larger than a single model's endpoints document.
const MAX_ZDR_BYTES: usize = 8 * 1024 * 1024;

/// Whether OpenRouter lists a provider route for the model at all. `Unknown`
/// means the check itself failed; it says nothing about the model.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RouteAvailability {
    Listed,
    Unlisted,
    Unknown,
}

/// What public endpoints say about retention for this model's routes. OpenRouter
/// does not publish normalized per-route data policy, so `ZdrListed` is only
/// positive evidence: every active route appears on the public ZDR list.
/// `Unknown` never implies that a provider retains data.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RoutePrivacy {
    ZdrListed,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModelCheck {
    pub availability: RouteAvailability,
    pub privacy: RoutePrivacy,
    /// Model variant suffix such as `batch`, when the identifier carries one.
    pub variant: Option<String>,
    /// Provider routes the endpoints list names for this model.
    pub routes: u32,
    /// Routes reporting an active status.
    pub active_routes: u32,
}

/// Full live text-model catalog for the "find another model" search. The
/// curated list stays unchanged; this only widens the search surface and also
/// refreshes the cached reasoning policy used by generation.
pub fn list_catalog(store: &Store, cancel: &AtomicBool) -> Result<Vec<AiModel>, String> {
    ai::supported()?;
    let value = ai::network(cancel, async {
        ai::request_json(
            ai::attribution(ai::client(Duration::from_secs(20))?.get(ai::MODELS_URL)),
            MAX_CATALOG_BYTES,
        )
        .await
    })?;
    ai::cache_model_policy(store, &value);
    ai::parse_models(value)
}

/// `author/slug[:variant]` → the public endpoints document. The variant stays
/// inside the final slug segment; `:` is legal in a URL path and is left intact.
fn endpoints_url(model: &str) -> Result<reqwest::Url, String> {
    let (author, slug) = model
        .split_once('/')
        .ok_or("Choose a model identifier from the OpenRouter catalog.")?;
    let mut url = reqwest::Url::parse(ai::MODELS_URL)
        .map_err(|_| "Cannot prepare the model check.")?;
    url.path_segments_mut()
        .map_err(|_| "Cannot prepare the model check.")?
        .extend([author, slug, "endpoints"]);
    Ok(url)
}

fn model_variant(model: &str) -> Option<String> {
    model.rsplit_once(':').map(|(_, variant)| variant.to_owned())
}

struct Route {
    tag: String,
    active: bool,
}

fn parse_endpoints(value: &serde_json::Value) -> Option<Vec<Route>> {
    let endpoints = value
        .get("data")?
        .get("endpoints")?
        .as_array()?;
    Some(
        endpoints
            .iter()
            .map(|endpoint| Route {
                tag: endpoint
                    .get("tag")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                // Status 0 marks a serving route; anything else is not
                // counted as usable.
                active: endpoint
                    .get("status")
                    .and_then(serde_json::Value::as_i64)
                    == Some(0),
            })
            .collect(),
    )
}

/// Public ZDR list: provider tags that currently accept zero-data-retention
/// requests for this model. Presence is positive evidence only.
fn zdr_tags(value: &serde_json::Value, model: &str) -> HashSet<String> {
    let list = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array());
    let mut tags = HashSet::new();
    for entry in list.into_iter().flatten() {
        let listed = entry
            .get("model_id")
            .or_else(|| entry.get("model"))
            .and_then(serde_json::Value::as_str);
        if listed != Some(model) {
            continue;
        }
        let status = entry.get("status").and_then(serde_json::Value::as_i64);
        if status.is_some_and(|status| status != 0) {
            continue;
        }
        if let Some(tag) = entry
            .get("tag")
            .or_else(|| entry.get("provider_tag"))
            .and_then(serde_json::Value::as_str)
        {
            tags.insert(tag.to_owned());
        }
    }
    tags
}

/// Public route check for one catalog model: endpoint availability plus the
/// limited privacy signal the public API offers. Nothing here sends the key,
/// user text, or an inference request.
pub fn check_model(model: &str, cancel: &AtomicBool) -> Result<AiModelCheck, String> {
    ai::supported()?;
    if !ai::valid_model(model) {
        return Err("Choose a model identifier from the OpenRouter catalog.".into());
    }
    let url = endpoints_url(model)?;
    // network() works in String errors; the status-preserving result rides in
    // the Ok side so a 404 stays distinguishable from a transport failure.
    let fetched = ai::network(cancel, async {
        Ok(ai::fetch_json(
            ai::attribution(ai::client(Duration::from_secs(15))?.get(url)),
            MAX_ENDPOINTS_BYTES,
        )
        .await)
    })?;
    let routes = match fetched {
        Ok(value) => match parse_endpoints(&value) {
            Some(routes) => routes,
            None => {
                return Ok(AiModelCheck {
                    availability: RouteAvailability::Unknown,
                    privacy: RoutePrivacy::Unknown,
                    variant: model_variant(model),
                    routes: 0,
                    active_routes: 0,
                })
            }
        },
        // A missing endpoints document means no listed route; any other
        // failure means the check itself could not run.
        Err(error) => {
            return Ok(AiModelCheck {
                availability: if error.status == Some(404) {
                    RouteAvailability::Unlisted
                } else {
                    RouteAvailability::Unknown
                },
                privacy: RoutePrivacy::Unknown,
                variant: model_variant(model),
                routes: 0,
                active_routes: 0,
            })
        }
    };
    let active: Vec<&Route> = routes.iter().filter(|route| route.active).collect();
    let availability = if routes.is_empty() {
        RouteAvailability::Unlisted
    } else {
        RouteAvailability::Listed
    };
    let privacy = if active.is_empty() {
        RoutePrivacy::Unknown
    } else {
        let zdr = ai::network(cancel, async {
            let client = ai::client(Duration::from_secs(10))?;
            Ok(ai::fetch_json(
                ai::attribution(client.get(ZDR_URL)),
                MAX_ZDR_BYTES,
            )
            .await)
        })
        .ok()
        .and_then(|inner| inner.ok())
        .map(|value| zdr_tags(&value, model));
        match zdr {
            // Positive evidence only: every active route is on the ZDR list.
            Some(tags) if !tags.is_empty()
                && active.iter().all(|route| tags.contains(&route.tag)) =>
            {
                RoutePrivacy::ZdrListed
            }
            _ => RoutePrivacy::Unknown,
        }
    };
    Ok(AiModelCheck {
        availability,
        privacy,
        variant: model_variant(model),
        routes: routes.len() as u32,
        active_routes: active.len() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_url_keeps_variant_in_the_slug_segment() {
        let url = endpoints_url("z-ai/glm-5.3-flash:batch").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("openrouter.ai"));
        assert_eq!(
            url.path(),
            "/api/v1/models/z-ai/glm-5.3-flash:batch/endpoints"
        );
        assert!(endpoints_url("no-slash").is_err());
    }

    #[test]
    fn endpoints_parse_counts_only_active_routes() {
        let routes = parse_endpoints(&serde_json::json!({
            "data": {"endpoints": [
                {"provider_name": "A", "tag": "a/fp8", "status": 0},
                {"provider_name": "B", "tag": "b", "status": -2},
                {"provider_name": "C", "tag": "c"}
            ]}
        }))
        .unwrap();
        assert_eq!(routes.len(), 3);
        assert_eq!(routes.iter().filter(|route| route.active).count(), 1);
        assert!(parse_endpoints(&serde_json::json!({"data": {}})).is_none());
        assert!(parse_endpoints(&serde_json::json!({})).is_none());
    }

    #[test]
    fn zdr_list_matches_model_and_active_tags_only() {
        let list = serde_json::json!({"data": [
            {"model_id": "a/b", "tag": "one", "status": 0},
            {"model_id": "a/b", "tag": "two", "status": -2},
            {"model_id": "a/b:batch", "tag": "three"},
            {"model_id": "other/model", "tag": "four", "status": 0}
        ]});
        let tags = zdr_tags(&list, "a/b");
        assert!(tags.contains("one"));
        assert!(!tags.contains("two"));
        assert!(!tags.contains("four"));
        let batch = zdr_tags(&list, "a/b:batch");
        assert!(batch.contains("three"));
    }

    #[test]
    fn variant_is_the_suffix_after_the_last_colon() {
        assert_eq!(model_variant("z-ai/glm-5.3-flash"), None);
        assert_eq!(model_variant("z-ai/glm-5.3-flash:batch").as_deref(), Some("batch"));
        assert_eq!(model_variant("provider/name:free").as_deref(), Some("free"));
    }
}
