use serde::{Deserialize, Serialize};
pub mod reports;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ContactInformationChange {
    SetGhost {
        id: String,
        palette: String,
        position_mm: [i32; 3],
    },
    RemoveGhost {
        id: String,
    },
    SetReportPolicy {
        target: String,
        policy: reports::ReportPolicy,
    },
    ClearReportPolicy {
        target: String,
    },
}

impl ContactInformationChange {
    pub fn target(&self) -> &str {
        match self {
            Self::SetGhost { id, .. } | Self::RemoveGhost { id } => id,
            Self::SetReportPolicy { target, .. } | Self::ClearReportPolicy { target } => target,
        }
    }
    pub fn bounded(&self) -> bool {
        let id = |v: &str| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control);
        id(self.target())
            && match self {
                Self::SetGhost { palette, .. } => id(palette),
                Self::SetReportPolicy { policy, .. } => policy.changes_report(),
                _ => true,
            }
    }
}
