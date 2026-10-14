use super::*;

#[test]
fn human_source_accepts_human_suppresses_ai_and_coordinates() {
    assert_eq!(
        control_tick_policy(ControlSource::Human),
        ControlTickPolicy {
            accept_human_input: true,
            accept_summary_input: false,
            operate_ai: false,
            coordinate: true,
        }
    );
}

#[test]
fn ai_source_suppresses_human_operates_ai_and_coordinates() {
    assert_eq!(
        control_tick_policy(ControlSource::Ai),
        ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: false,
            operate_ai: true,
            coordinate: true,
        }
    );
}

#[test]
fn resolver_defaults_to_human_per_instance() {
    let resolver = ControlSourceResolver::new();

    assert_eq!(
        resolver.source_for(&SystemId("helm".into())),
        ControlSource::Human
    );
}

#[test]
fn resolver_selects_source_per_system_instance() {
    let mut resolver = ControlSourceResolver::new();
    let helm = SystemId("helm".into());
    let red_alert = SystemId("red-alert".into());

    resolver.set(helm.clone(), ControlSource::Ai);

    assert_eq!(resolver.source_for(&helm), ControlSource::Ai);
    assert_eq!(resolver.source_for(&red_alert), ControlSource::Human);
    assert_eq!(
        resolver.policy_for(&helm),
        ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: false,
            operate_ai: true,
            coordinate: true,
        }
    );
}

#[test]
fn offline_returns_false_false_false_policy() {
    assert_eq!(
        control_tick_policy(ControlSource::Offline),
        ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: false,
            operate_ai: false,
            coordinate: false,
        }
    );
}

#[test]
fn offline_systems_gate_overrides_human_and_ai() {
    let mut resolver = ControlSourceResolver::new();
    let helm = SystemId("helm".into());
    let tactical = SystemId("tactical".into());

    // helm is Human-controlled, tactical is Ai-controlled.
    resolver.set(helm.clone(), ControlSource::Human);
    resolver.set(tactical.clone(), ControlSource::Ai);

    // Mark both as offline via damage.
    resolver.set_offline(helm.clone(), true);
    resolver.set_offline(tactical.clone(), true);
    assert!(resolver.is_offline(&helm));
    assert!(resolver.is_offline(&tactical));

    // Both must return the offline policy, regardless of ControlSource.
    let offline_policy = ControlTickPolicy {
        accept_human_input: false,
        accept_summary_input: false,
        operate_ai: false,
        coordinate: false,
    };
    assert_eq!(resolver.policy_for(&helm), offline_policy);
    assert_eq!(resolver.policy_for(&tactical), offline_policy);
}

#[test]
fn restore_from_offline_set_restores_original_policy() {
    let mut resolver = ControlSourceResolver::new();
    let helm = SystemId("helm".into());

    // Human-controlled, then marked offline.
    resolver.set(helm.clone(), ControlSource::Human);
    resolver.set_offline(helm.clone(), true);

    // Offline.
    assert_eq!(
        resolver.policy_for(&helm),
        ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: false,
            operate_ai: false,
            coordinate: false,
        }
    );

    // Repair: remove from offline set.
    resolver.set_offline(helm.clone(), false);
    assert!(!resolver.is_offline(&helm));

    // Human policy restored.
    assert_eq!(
        resolver.policy_for(&helm),
        ControlTickPolicy {
            accept_human_input: true,
            accept_summary_input: false,
            operate_ai: false,
            coordinate: true,
        }
    );
}

#[test]
fn replacing_offline_systems_clears_bootstrap_entries() {
    let mut resolver = ControlSourceResolver::new();
    let bootstrap = SystemId("bootstrap".into());
    let captured = SystemId("captured".into());
    resolver.set_offline(bootstrap.clone(), true);

    resolver.replace_offline_systems([captured.clone()]);

    assert!(!resolver.is_offline(&bootstrap));
    assert!(resolver.is_offline(&captured));
    assert_eq!(resolver.offline_entries().count(), 1);
}

#[test]
fn crew_planning_uses_ordinary_controller_despite_failed_tools() {
    let id = SystemId("repair-orders".into());
    let mut resolver = ControlSourceResolver::new();
    resolver.set_offline(id.clone(), true);
    assert!(!resolver.policy_for(&id).accept_human_input);
    assert!(resolver.crew_order_policy_for(&id).accept_human_input);
    resolver.set(id.clone(), ControlSource::Ai);
    assert!(!resolver.policy_for(&id).operate_ai);
    assert!(resolver.crew_order_policy_for(&id).operate_ai);
    resolver.set(id.clone(), ControlSource::Simplified);
    let policy = resolver.crew_order_policy_for(&id);
    assert!(policy.operate_ai && policy.accept_summary_input && !policy.accept_human_input);
}

#[test]
fn crew_orders_do_not_override_gm_explicit_offline_or_catastrophe() {
    let id = SystemId("repair-orders".into());
    let mut resolver = ControlSourceResolver::new();
    resolver.set(id.clone(), ControlSource::Ai);
    resolver.set_offline(id.clone(), true);
    resolver.set_gm_disabled(id.clone(), true);
    assert!(!resolver.crew_order_policy_for(&id).operate_ai);
    resolver.set_gm_disabled(id.clone(), false);
    resolver.set(id.clone(), ControlSource::Offline);
    assert!(!resolver.crew_order_policy_for(&id).coordinate);
    resolver.set(id.clone(), ControlSource::Human);
    resolver.set_destroyed(true);
    assert!(!resolver.crew_order_policy_for(&id).accept_human_input);
    resolver.set_destroyed(false);
    assert!(resolver.crew_order_policy_for(&id).accept_human_input);
    assert!(
        !resolver.policy_for(&id).accept_human_input,
        "planning availability never makes the failed physical tools operational"
    );
}
