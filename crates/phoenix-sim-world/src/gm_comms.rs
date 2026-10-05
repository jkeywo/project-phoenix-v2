use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GmCommsVisibility {
    SelectedShips,
    Fleet,
}

/// One explicitly authored root callable through ordinary Comms materialisation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmCommsHail {
    pub id: String,
    pub label: String,
    pub script_path: String,
    pub root_fn: String,
}

/// Root-world authored routing choices. No route is fabricated for old worlds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmCommsRoute {
    pub id: String,
    pub label: String,
    pub visibility: GmCommsVisibility,
    /// Existing authored entity reference names; resolved to live UUIDs.
    pub senders: Vec<String>,
    #[serde(default, rename = "hail")]
    pub hails: Vec<GmCommsHail>,
    /// Optional Game Master attention band for pending conversations this
    /// route's speakers hold open (issue #1433). One of `urgent`, `attention`
    /// or `background`; absent means the system default, `attention`.
    ///
    /// Held as the authored string rather than the typed
    /// [`crate::gm_attention::GmAttentionBand`] on purpose: a mistyped band is
    /// a *world* error that has to name the `[[gm_comms_route]]` it came from
    /// (see [`validate_routes`]), and a serde variant failure names a TOML path
    /// instead. GM-facing triage only — it changes no crew delivery, routing,
    /// recipients or `CommsPriority`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_band: Option<String>,
}

pub fn validate_routes(routes: &[GmCommsRoute]) -> Result<(), String> {
    let valid =
        |id: &str| !id.trim().is_empty() && id.len() <= 128 && !id.chars().any(char::is_control);
    let mut ids = std::collections::BTreeSet::new();
    for route in routes {
        if !valid(&route.id)
            || !ids.insert(&route.id)
            || !valid(&route.label)
            || route.senders.is_empty()
            || route.senders.iter().any(|s| !valid(s))
        {
            return Err(format!(
                "invalid or duplicate [[gm_comms_route]] '{}'",
                route.id
            ));
        }
        // The GM attention band (issue #1433) is a closed three-word
        // vocabulary. A world that names a fourth is refused at load with the
        // section and id that carry it, rather than quietly falling back to the
        // default and shipping a scenario whose triage nobody chose.
        if let Some(band) = route.attention_band.as_deref() {
            if crate::gm_attention::GmAttentionBand::from_authored(band).is_none() {
                return Err(format!(
                    "[[gm_comms_route]] '{}' declares attention_band '{band}'; the \
                     Game Master attention queue accepts only {}",
                    route.id,
                    crate::gm_attention::GmAttentionBand::authored_vocabulary()
                ));
            }
        }
        let mut hails = std::collections::BTreeSet::new();
        for hail in &route.hails {
            if !valid(&hail.id)
                || !hails.insert(&hail.id)
                || !valid(&hail.label)
                || !valid(&hail.script_path)
                || !valid(&hail.root_fn)
            {
                return Err(format!("invalid or duplicate GM Comms hail '{}'", hail.id));
            }
        }
    }
    Ok(())
}
