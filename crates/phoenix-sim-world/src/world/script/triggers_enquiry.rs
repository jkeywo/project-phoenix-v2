//! Observer-aware enquiry registration through the ordinary trigger builder.
use super::*;
use crate::world::config::{valid_enquiry_aim, EnquiryPhase};
use phoenix_model::identity::SystemId;
use rhai::Map;

fn optional_text(options: &mut Map, key: &str) -> Result<Option<String>, String> {
    options
        .remove(key)
        .map(|value| {
            let text = value
                .try_cast::<ImmutableString>()
                .ok_or_else(|| format!("on_enquiry {key} must be text"))?;
            if text.is_empty() || text.len() > 128 || text.chars().any(char::is_control) {
                return Err(format!(
                    "on_enquiry {key} must be 1..=128 bytes of control-free text"
                ));
            }
            Ok(text.to_string())
        })
        .transpose()
}

fn condition(entity: &str, mut options: Map) -> Result<TriggerCondition, String> {
    if entity.is_empty() || entity.len() > 128 || entity.chars().any(char::is_control) {
        return Err("on_enquiry target must be 1..=128 bytes of control-free text".into());
    }
    let observer_name = optional_text(&mut options, "observer")?;
    let system = optional_text(&mut options, "system")?.map(SystemId);
    let aim = optional_text(&mut options, "aim")?;
    if aim
        .as_deref()
        .is_some_and(|value| !valid_enquiry_aim(value))
    {
        return Err(
            "on_enquiry aim must be general, environment, resources, anomaly or shields".into(),
        );
    }
    let phase = optional_text(&mut options, "phase")?
        .map(|value| {
            EnquiryPhase::parse(&value)
                .ok_or_else(|| "on_enquiry phase must be started or completed".to_owned())
        })
        .transpose()?
        .unwrap_or(EnquiryPhase::Completed);
    let intrusive = options
        .remove("intrusive")
        .map(|value| {
            value
                .try_cast::<bool>()
                .ok_or_else(|| "on_enquiry intrusive must be boolean".to_owned())
        })
        .transpose()?;
    if !options.is_empty() {
        return Err("on_enquiry contains unsupported filter fields".into());
    }
    Ok(TriggerCondition::OnEnquiry {
        entity_name: entity.into(),
        observer_name,
        system,
        aim,
        intrusive,
        phase,
    })
}

pub(super) fn register(engine: &mut HostRegistry, state: Arc<Mutex<BuilderState>>) {
    let s = state.clone();
    host_fn!(engine, "on_enquiry", receiver = "", category = "trigger",
        params = ["entity", "handler"],
        summary = "Fire on completed paid focused enquiry of an entity. Optional middle map filters observer, system, aim, intrusive and phase.",
        move |entity: ImmutableString, handler: ImmutableString| -> Result<TriggerHandle, Box<EvalAltResult>> {
            Ok(push_trigger(&s, condition(&entity, Map::new()).map_err(raise)?, &handler))
        },
    );
    engine.register_fn(
        "on_enquiry",
        move |entity: ImmutableString,
              options: Map,
              handler: ImmutableString|
              -> Result<TriggerHandle, Box<EvalAltResult>> {
            Ok(push_trigger(
                &state,
                condition(&entity, options).map_err(raise)?,
                &handler,
            ))
        },
    );
}
