//! Campaign continuity (issue #867, parent #848).
//!
//! One mission ends and leaves a save; the next one opens knowing some of what
//! happened. [`projection`] is the whole of that seam — a pure fold from a
//! finished mission's `vellum-save` snapshot to the facts a campaign is allowed
//! to carry — and there is deliberately nothing else in this module.
//!
//! No runner, no store, no per-campaign state. A campaign that wants to keep
//! these facts writes them down through the same `vellum_save::Store` a save
//! goes through (issue #866), and a mission that wants to read them reads
//! ordinary world flags. What was missing was the *contract* saying which facts
//! survive, and that is a pure function and a struct.

/// The pure fold: the declared fact vocabulary, and the rule that keeps
/// transient combat state out by leaving it nowhere to go.
pub mod projection;

pub use projection::{
    project, seed_flags, CampaignAsset, CampaignFacts, CampaignFinding, CampaignPromise,
    CampaignStanding, CampaignStructure, CAMPAIGN_FACTS_VERSION, CAMPAIGN_FLAG_PREFIX,
};
