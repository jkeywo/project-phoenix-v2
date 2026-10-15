//! Closed provider terms. Goods remain in existing actual resource endpoints.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortServicePolicy {
    FreeAlliedReplenishment,
    ManualOffer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortServiceConfig {
    pub policy: PortServicePolicy,
    /// Explicit membership permission. Empty is deny-all, never everyone.
    pub eligible_factions: Vec<Uuid>,
    #[serde(rename = "offer")]
    pub offers: Vec<PortOfferConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOfferConfig {
    pub id: String,
    pub resource: String,
    /// Maximum whole lots delivered to one visitor in one physical visit.
    /// Zero disables this offer, including inbound donations. Positive-quota
    /// free donations do not consume its outbound delivered-lot allowance.
    pub quota: i64,
    /// Actual resource units delivered per whole lot.
    pub unit_amount: i64,
    #[serde(default, rename = "cost")]
    pub costs: Vec<PortCost>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortCost {
    pub resource: String,
    /// Actual existing stock units consumed per whole lot, not currency.
    pub amount: i64,
}

/// The human approves this complete bounded line, including its receiving store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortOrderLine {
    pub offer: String,
    pub store: String,
    pub lots: i64,
}

impl PortServiceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.eligible_factions.iter().collect::<BTreeSet<_>>().len()
            != self.eligible_factions.len()
        {
            return Err("port eligible faction UUIDs must be unique".into());
        }
        if self.offers.is_empty() {
            return Err("port service requires at least one explicit offer".into());
        }
        let mut ids = BTreeSet::new();
        let mut resources = BTreeSet::new();
        for offer in &self.offers {
            if offer.id.trim().is_empty()
                || offer.resource.trim().is_empty()
                || !ids.insert(&offer.id)
                || !resources.insert(&offer.resource)
                || offer.quota < 0
                || offer.unit_amount <= 0
                || offer.quota.checked_mul(offer.unit_amount).is_none()
            {
                return Err("port offers require unique identities/resources, nonnegative lot quota and positive bounded units per lot".into());
            }
            if self.policy == PortServicePolicy::FreeAlliedReplenishment && !offer.costs.is_empty()
            {
                return Err("free-allied port offers cannot spend visitor stocks".into());
            }
            let mut costs = BTreeSet::new();
            for cost in &offer.costs {
                if cost.resource.trim().is_empty()
                    || !costs.insert(&cost.resource)
                    || cost.amount <= 0
                    || offer.quota.checked_mul(cost.amount).is_none()
                {
                    return Err("port costs require unique named resources and positive bounded stock units per lot".into());
                }
            }
        }
        Ok(())
    }

    pub fn offer(&self, id: &str) -> Option<&PortOfferConfig> {
        self.offers.iter().find(|offer| offer.id == id)
    }
}

#[cfg(test)]
#[path = "ports_tests.rs"]
mod tests;
