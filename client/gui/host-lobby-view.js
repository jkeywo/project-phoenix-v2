/**
 * gui/host-lobby-view.js — Pure view-model behind server.html's
 * `__updateLobby()` (issue #1229).
 *
 * Computes everything the host's lobby/Station-grid renderer decides (phase
 * transitions, title/subtitle, crew counter, ready badge, countdown, station
 * cards, the reserved-slots chip, the spectator pill list, the status hint,
 * and the AI-only launch button) from the Rust-pushed `LobbyStatePayload`
 * JSON (`src/core/messages.rs`) plus the previously-seen phase. All DOM
 * writes — and all side effects (audio, wake-state variables) — stay in
 * server.html's inline glue, which consumes this view model.
 *
 * SIBLING of gui/lobby-view.js, not a reuse of it: the host consumes a
 * Rust-built roster whose station rows already carry resolved display text
 * (`name`/`short_code`/`rank`/`holder_name`/`preset_names` — every host
 * channel push crosses `localiseHostPayload`, issue #949) and there is no
 * `myToken` — this renders the Viewscreen's read-only card grid with crew
 * dots, not the phone's claim/release rows. Do not attempt to unify the two
 * modules or change the Rust lobby payload shape.
 *
 * Text that still needs localisation at render time (badge/hint/pill
 * strings whose id depends on the data) is returned as `{ id, params }`
 * pairs, exactly like gui/lobby-view.js's `statusLine` — the glue resolves
 * them through `t()`. Plain data already resolved by the host (station
 * names, holder names, ranks) passes through as strings.
 */

/**
 * @param {object} s  Parsed `LobbyStatePayload` — { phase, scenario_title,
 *                    scenario_body, crew_count, max_players, all_ready,
 *                    stations: [{ name, short_code, rank, holder_name,
 *                    preset_names, consoles? }], spectators: string[],
 *                    loading_progress?: number, countdown_secs }.
 * @param {string} prevPhase  The phase seen on the previous call (server.html's
 *                    `_lobbyPrevPhase`), used to detect the Loading→InProgress
 *                    and "entered InProgress" edges.
 * @returns {object} view model — see the return literal below.
 */
export function hostLobbyViewModel(s, prevPhase) {
  const phase = s.phase;
  const isLobby = phase === 'Lobby';
  const maxP = s.max_players || 0;
  const crewN = s.crew_count || 0;
  const stations = s.stations || [];
  const spectators = s.spectators || [];
  const countdownSecs = s.countdown_secs || 0;

  // ── Phase transitions (loading overlay, audio, panel/QR visibility) ────
  // Pure decisions only — starting/stopping audio and touching the shared
  // `qrVisible` classic-script variable are side effects the glue performs.
  const showLoadingOverlay = phase === 'Loading';
  const loadingPct = (showLoadingOverlay && typeof s.loading_progress === 'number')
    ? Math.round(s.loading_progress * 100) + '%'
    : null;
  // Only this edge dismisses the loading overlay — there's nothing to
  // dismiss if it was never shown.
  const dismissLoadingOverlay = phase === 'InProgress' && prevPhase === 'Loading';
  // Audio unlocks on ANY entry into InProgress, not just from Loading: when
  // the asset preload is already complete, a direct start sets InProgress
  // without ever passing through Loading.
  const unlockAudio = phase === 'InProgress' && prevPhase !== 'InProgress';
  const menuMusic = phase === 'Lobby' ? 'start'
    : (phase === 'InProgress' || phase === 'Loading') ? 'stop'
    : null;
  // During InProgress the settings menus are the sole QR-overlay controllers
  // — this transition leaves it untouched (null).
  const qrOverlayAction = isLobby ? 'show'
    : (phase === 'Loading' || phase === 'GameOver') ? 'hide'
    : null;
  const hideGameOverOverlay = phase !== 'GameOver';

  const transitions = {
    phase,
    prevPhase,
    showLoadingOverlay,
    loadingPct,
    dismissLoadingOverlay,
    unlockAudio,
    menuMusic,
    showPanel: isLobby,
    qrOverlayAction,
    hideGameOverOverlay,
  };

  // ── Title / subtitle ────────────────────────────────────────────────────
  // Both are the world's authored title/description, already resolved
  // strings on the wire. `title` collapses to null (falsy) on an empty
  // string so the glue's `vm.title || t('server.unknown_scenario')` fallback
  // matches the original `s.scenario_title || t(...)` exactly.
  const title = s.scenario_title || null;
  const subtitle = s.scenario_body || '';

  // ── Crew counter + spectator tag ────────────────────────────────────────
  const hasSpecs = spectators.length > 0;
  const crew = {
    count: crewN,
    max: maxP,
    dots: Array.from({ length: maxP }, (_, i) => i < crewN),
    spectatorTag: { visible: hasSpecs, count: spectators.length },
  };

  // ── Ready badge ──────────────────────────────────────────────────────────
  let readyBadge;
  if (countdownSecs > 0) {
    readyBadge = { id: 'server.launching_in', params: { secs: countdownSecs }, className: 'go' };
  } else if (s.all_ready) {
    readyBadge = { id: 'client.all_crew_ready', params: {}, className: 'go' };
  } else {
    readyBadge = { id: 'client.awaiting_crew', params: {}, className: '' };
  }

  // ── Countdown display ────────────────────────────────────────────────────
  const countdown = { visible: countdownSecs > 0, secs: countdownSecs };

  // ── Station grid ─────────────────────────────────────────────────────────
  // The grid shows exactly the ship's defined station roster — no padding to
  // a fixed slot count — so every card is populated; there is no empty-slot
  // variant to compute.
  const cards = stations.map(st => {
    const claimed = !!st.holder_name;
    const avatar = claimed
      ? { text: st.holder_name.substring(0, 2).toUpperCase(), placeholder: false }
      : { text: st.short_code ? st.short_code.substring(0, 2).toUpperCase() : '--', placeholder: true };
    const consoles = st.consoles && st.consoles.length > 0 ? st.consoles : [];
    const presetPills = (st.preset_names && st.preset_names.length > 0)
      ? st.preset_names.map(pn => ({
          low: pn === 'Low',
          id: pn === 'Low' ? 'server.complexity_low' : 'server.complexity_normal',
        }))
      : [];
    return {
      claimed,
      name: st.name || st.short_code || '',
      rank: st.rank || '',
      avatar,
      consoles,
      presetPills,
    };
  });

  // ── Reserved-slots chip ──────────────────────────────────────────────────
  // The grid is sized to the roster (MAX_SLOTS === stations.length), so this
  // is always 0 today; the arithmetic is kept in the shape the original
  // fixed-count grid used, in case a future padded layout revives it.
  const MAX_SLOTS = stations.length;
  const reservedCount = MAX_SLOTS - stations.length;
  const reservedChip = reservedCount > 0
    ? {
        active: true,
        id: reservedCount === 1 ? 'server.slots_reserved.one' : 'server.slots_reserved.other',
        params: { n: reservedCount, max: MAX_SLOTS },
      }
    : { active: false, id: null, params: {} };

  // ── Spectator pill list ──────────────────────────────────────────────────
  const crewPills = stations
    .filter(st => st.holder_name)
    .map(st => ({ kind: 'crew', text: `${st.holder_name} · ${st.name}` }));
  const waitingPills = spectators.map(name => ({
    kind: 'waiting',
    id: 'server.spectator_waiting',
    params: { name },
  }));
  const spectatorPills = crewPills.length === 0 && waitingPills.length === 0
    ? [{ kind: 'empty', id: 'server.no_players', params: {} }]
    : [...crewPills, ...waitingPills];

  // ── Status hint ──────────────────────────────────────────────────────────
  let hint;
  if (countdownSecs > 0) {
    hint = { id: 'server.hint_launching', params: { secs: countdownSecs }, color: '#5fd8e8' };
  } else if (crewN === 0) {
    hint = { id: 'server.waiting_players', params: {}, color: '#667' };
  } else if (s.all_ready) {
    hint = { id: 'client.status_all_ready', params: {}, color: '#5fd8e8' };
  } else {
    hint = { id: 'server.waiting_ready', params: {}, color: '#889' };
  }

  // ── AI-only launch button ────────────────────────────────────────────────
  const aiLaunchVisible = crewN === 0 && spectators.length === 0;

  return {
    transitions,
    title,
    subtitle,
    crew,
    readyBadge,
    countdown,
    cards,
    reservedChip,
    spectatorPills,
    hint,
    aiLaunchVisible,
  };
}

// Expose for the classic (non-module) script in server.html.
if (typeof window !== 'undefined') {
  window.hostLobbyViewModel = hostLobbyViewModel;
}
