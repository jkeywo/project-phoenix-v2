//! Public fixed XYZ points. Source publications remain separate knowledge scopes.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationPointView {
    pub id: String,
    pub label: String,
    pub point: [f32; 3],
}
impl NavigationPointView {
    pub fn validate(&self) -> Result<(), String> {
        // Keep the established TransitRouteView acceptance and wire shape.
        let identity = |value: &str| {
            !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        };
        if !identity(&self.id)
            || !identity(&self.label)
            || !self.point.iter().all(|value| value.is_finite())
        {
            return Err("invalid known authored Transit route".into());
        }
        Ok(())
    }
}

/// The original admitted fixed chart point, not a live actor tracker or a Job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartedNavigationCourse {
    pub owner: SystemId,
    pub destination: NavigationPointView,
    pub selected_tick: u64,
}
impl ChartedNavigationCourse {
    pub fn valid_at(&self, tick: u64) -> bool {
        !self.owner.0.trim().is_empty()
            && self.owner.0.len() <= 512
            && !self.owner.0.chars().any(char::is_control)
            && self.selected_tick <= tick
            && self.destination.validate().is_ok()
    }
    pub fn same_intent(&self, other: &Self) -> bool {
        self.owner == other.owner && self.destination == other.destination
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_value_keeps_existing_transit_wire_shape_and_finite_acceptance() {
        let json = r#"{"id":"local-id","label":"public.name","point":[1.25,18.5,-23.75]}"#;
        let route: crate::transit::TransitRouteView = serde_json::from_str(json).unwrap();
        let point: NavigationPointView = serde_json::from_str(json).unwrap();
        assert_eq!(route, point);
        assert_eq!(
            serde_json::to_value(&route).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
        assert!(point.validate().is_ok());
        for coordinate in [f32::NAN, f32::INFINITY] {
            let mut invalid = point.clone();
            invalid.point[1] = coordinate;
            assert!(invalid.validate().is_err());
        }
        let mut control = point.clone();
        control.label = "bad\nlabel".into();
        assert!(control.validate().is_err());
    }
}
