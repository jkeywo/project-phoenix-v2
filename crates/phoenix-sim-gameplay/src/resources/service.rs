//! Explicit consumer links to local resource endpoints. No provider search or stock state.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use phoenix_model::logistics::ResourceEndpoint;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ResourceEndpoint>,
}

impl ResourceLink {
    pub fn is_empty(&self) -> bool {
        self.service.is_none() && self.provider.is_none()
    }
}

pub fn resolve_endpoint(
    resource: &str,
    services: &BTreeMap<String, ResourceEndpoint>,
    link: &ResourceLink,
) -> Result<ResourceEndpoint, String> {
    if let Some(service) = &link.service {
        if service.trim().is_empty() {
            return Err("resource link service must have a non-empty name".into());
        }
    }
    let endpoint = if let Some(provider) = &link.provider {
        provider.clone()
    } else if let Some(service) = &link.service {
        services
            .get(service)
            .cloned()
            .ok_or_else(|| format!("resource service {service:?} is not declared on this entity"))?
    } else {
        services
            .get(resource)
            .cloned()
            .unwrap_or_else(|| ResourceEndpoint::Capacity {
                id: resource.into(),
            })
    };
    endpoint.validate_resource(resource)?;
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_services_explicit_overrides_and_no_search() {
        let services = BTreeMap::from([
            ("torpedoes".into(), ResourceEndpoint::TorpedoMagazine {}),
            (
                "fuel".into(),
                ResourceEndpoint::Capacity {
                    id: "reserve".into(),
                },
            ),
        ]);
        assert_eq!(
            resolve_endpoint("torpedoes", &services, &ResourceLink::default()).unwrap(),
            ResourceEndpoint::TorpedoMagazine {}
        );
        assert_eq!(
            resolve_endpoint("other", &services, &ResourceLink::default()).unwrap(),
            ResourceEndpoint::Capacity { id: "other".into() }
        );
        let missing = ResourceLink {
            service: Some("absent".into()),
            provider: None,
        };
        assert!(resolve_endpoint("fuel", &services, &missing)
            .unwrap_err()
            .contains("not declared"));
        let override_ = ResourceLink {
            provider: Some(ResourceEndpoint::Capacity { id: "other".into() }),
            ..missing
        };
        assert_eq!(
            resolve_endpoint("fuel", &services, &override_).unwrap(),
            ResourceEndpoint::Capacity { id: "other".into() }
        );
        assert!(resolve_endpoint(
            "fuel",
            &services,
            &ResourceLink {
                provider: Some(ResourceEndpoint::TorpedoMagazine {}),
                service: None
            }
        )
        .is_err());
    }
    #[test]
    fn tagged_endpoints_reject_unknown_fields() {
        assert!(
            toml::from_str::<ResourceEndpoint>("kind='torpedo_magazine'\nid='leftover'").is_err()
        );
        assert!(toml::from_str::<ResourceLink>("search=true").is_err());
    }
    #[test]
    fn authored_services_require_real_local_endpoints_but_alias_names_are_supported() {
        use crate::entities::config::EntityConfig;
        let valid = r#"
[torpedoes]
count = 3
[torpedoes.ai]
idle = true
[resource_services.reload]
kind = "torpedo_magazine"
[resource_services.fuel]
kind = "capacity"
id = "reserve"
[[infrastructure.capacity]]
id = "reserve"
amount = 10
"#;
        let config = EntityConfig::from_toml(valid).unwrap();
        assert_eq!(
            resolve_endpoint(
                "torpedoes",
                &config.resource_services,
                &ResourceLink {
                    service: Some("reload".into()),
                    provider: None
                }
            )
            .unwrap(),
            ResourceEndpoint::TorpedoMagazine {}
        );
        assert!(
            EntityConfig::from_toml("[resource_services.fuel]\nkind='capacity'\nid='missing'")
                .unwrap_err()
                .to_string()
                .contains("not declared")
        );
        assert!(
            EntityConfig::from_toml("[resource_services.reload]\nkind='torpedo_magazine'")
                .unwrap_err()
                .to_string()
                .contains("requires [torpedoes]")
        );
    }
}
