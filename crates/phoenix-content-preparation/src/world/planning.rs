//! Accepted-source HTN capability checks over real offered hull recipes.
use super::{
    activity_bindings,
    validate::{WorldFinding, WorldSource},
};
use crate::entities::loader::TemplateLoader;

pub(super) fn findings(source: &WorldSource, loader: &dyn TemplateLoader) -> Vec<WorldFinding> {
    let mut findings = super::opportunities::findings(source, loader);
    findings.extend(planning_findings(source, loader));
    findings
}
fn planning_findings(source: &WorldSource, loader: &dyn TemplateLoader) -> Vec<WorldFinding> {
    let Some(authored) = &source.config.planning else {
        return Vec::new();
    };
    let check = || -> Result<(), String> {
        let compiled = source.config.compiled_planning.as_ref().ok_or(
            "planning imports must be resolved by the World content reader before activation",
        )?;
        if compiled.config() != authored {
            return Err("compiled planning does not match its authored root".into());
        }
        let actor = compiled.actor_reference()?;
        source.config.validate_situation_actor(actor)?;
        let prefix = format!("htn/{}/", authored.id);
        if source
            .config
            .situations
            .iter()
            .any(|row| row.id.starts_with(&prefix))
        {
            return Err(
                "authored situations cannot occupy the imported plan's reserved instance prefix"
                    .into(),
            );
        }
        for config in activity_bindings::candidates(actor, source, loader)? {
            let selected = compiled.choose("accepted-source-preview",0,|request| {
                if activity_bindings::supports(request,&config)? {Ok(())} else {Err("primitive is outside this offered hull's actual Repair visit/team/remedy or delivery route".into())}
            })?;
            // Reserve the same finite World owner capacity across explicit and
            // imported bindings. No leaf is silently dropped to fit.
            let mut instances = phoenix_sim_world::world::activities::SituationInstances::default();
            for row in &source.config.situations {
                let definition = source
                    .config
                    .situation_definitions
                    .iter()
                    .find(|definition| definition.id == row.definition)
                    .ok_or("unknown authored situation definition")?;
                instances.arm(row, definition, 0)?;
            }
            for row in selected.graph.leaves.values() {
                if let phoenix_sim_world::world::activities::ActivityRequest::Communication {
                    recipient,
                    ..
                } = &row.request
                {
                    if recipient.starts_with("slot:") {
                        return Err("first communication activity recipient must be a declared base-world actor".into());
                    }
                    source.config.validate_situation_actor(recipient)?;
                    for target in activity_bindings::candidates(recipient, source, loader)? {
                        if target
                            .comms
                            .as_ref()
                            .and_then(|row| row.radio.as_ref())
                            .is_none()
                        {
                            return Err("communication activity recipient requires actual supported radio equipment".into());
                        }
                    }
                }
                let definition = selected
                    .graph
                    .definitions
                    .get(&row.definition)
                    .ok_or("missing compiled primitive Scenario definition")?;
                instances.arm(row, definition, 0)?;
            }
        }
        Ok(())
    };
    match check() {
        Ok(()) => Vec::new(),
        Err(error) if !loader.absence_is_final() && error.starts_with("cannot resolve") => {
            Vec::new()
        }
        Err(error) => vec![WorldFinding::error(
            "unsupported-maintenance-plan",
            &source.path,
            source.toml,
            "planning",
            error,
        )],
    }
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;
