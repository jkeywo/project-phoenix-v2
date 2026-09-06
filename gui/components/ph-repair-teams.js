// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  REPAIR_DISPATCH_ACTION_ID,
  REPAIR_PRIORITY_ACTION_ID,
  REPAIR_RECALL_ACTION_ID,
} from '../stations/engineering-actions.js';
import { EXTERNAL_REPAIR_TARGET } from '../repair-dispatch.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * `ph-repair-teams` — the repair roster as a list of SELECTABLE cards
 * (issue #1384, PRD #1371).
 *
 * ## One card open at a time
 *
 * Every team used to render its whole control surface at once — a row of
 * dispatch buttons per idle team — under a separate, permanently visible
 * damaged-systems list. On a phone that is three panels of controls competing
 * for one column, most of them for teams the player is not thinking about.
 *
 * Now the roster is a summary (name, status, progress) and the player SELECTS
 * the team they mean; that card opens and the verbs appear inside it:
 *
 *   - **idle** → DISPATCH TO: one destination per damageable station, Core,
 *     and — on a hull that authored `[repair.external_dispatch]` — the field
 *     target off the ship.
 *   - **on site** (`repairing`) → RECALL, plus the damaged systems the host is
 *     willing to show this seat, tappable to become that team's next job. That
 *     list used to sit below the roster; it now belongs to the card whose team
 *     can actually act on it.
 *   - **travelling** → RECALL (issue #1385). A team that has been sent to the
 *     wrong station is the one thing a seat wants to undo, and undoing it early
 *     is cheap: the walk back is only as long as the walk out so far.
 *   - **abroad** (issue #1386) → the team the crew sent to the field target:
 *     RECALL, the target it is working, and the TARGET'S OWN condition track as
 *     the bar. A team abroad has no travel or repair progress of its own to show
 *     — it is already there — so the honest thing to draw is the work it is
 *     doing. This is a CLIENT-derived status: the wire slot stays `Idle`,
 *     because the team never walked anywhere on this hull, and the repair
 *     blackboard names which slot the claim holds.
 *   - **returning** → status and progress only. It is already coming home, so
 *     the host refuses a recall, and a button that sends nothing is worse than
 *     no button.
 *
 * The same rule governs the summary line itself: a card with no body to open
 * onto is not offered as a toggle at all — its summary is a disabled readout
 * with no caret, rather than a focusable control that rotates a caret and
 * reports `aria-expanded="true"` over a region that stays hidden.
 *
 * Selection is CLIENT-LOCAL and deliberately not a semantic action: it steers
 * no command, the host has no opinion about it, and a round trip would make
 * opening a card cost a frame of latency. It survives state pushes and is
 * dropped when the team it names leaves the roster.
 *
 * ## What this component does not decide
 *
 * `state.damaged` is `RepairConsolePayload.damaged_systems` — already the
 * host's own answer to "what may this seat see": core detail, this station's
 * systems, and whatever a team is standing on right now
 * (`src/console/repair/visibility.rs`). It is a per-SYSTEM projection, not a
 * per-team one, so the on-site card renders it whole rather than filtering it
 * by team — the console has no honest way to say which on-site team owns which
 * row, and `prioritisable` is the host's own verdict on whether a tap can do
 * anything at all.
 */
export class PhRepairTeams extends PhElement {
  // Own state accessors kept (not the base's): `set state` also kicks the
  // progress-bar animation loop, which the base setter has no hook for.
  #state = null;
  #animFrame = null;
  #displayProgress = new Map();
  #emptyEl = null;
  // Which team's card is open, or null. Client-local presentation state.
  #selectedTeamId = null;

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; gap: 0.5rem; min-height: 0; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .header { flex-shrink: 0; display: flex; justify-content: space-between; align-items: center; font-size: var(--text-sm); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .auto-badge { font-size: var(--text-xs); color: var(--reloading); border: 1px solid var(--reloading); padding: 0.05rem 0.3rem; letter-spacing: 0.2em; }
    /* The roster scrolls INSIDE the component, so an open card — which is
       taller than the summary it replaces — never pushes the console's own
       column past its grid cell and onto the panels below it. The heading
       stays put; only the cards move. Where the host is not height-bounded
       (a hull that lets the panel size itself) this is inert. */
    #teams-container { flex: 1 1 auto; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 0.4rem; }
    .card { flex-shrink: 0; border: 1px solid var(--line-faint); background: var(--bg-card); padding: 0.5rem; display: flex; flex-direction: column; gap: 0.3rem; }
    .card.selected { border-color: var(--cyan); }
    /* The whole summary line is the selector: a phone thumb gets the entire
       width of the card rather than a chevron-sized target, and the button
       carries the team's own name so its accessible name needs no second
       label. Reset to look like the row it replaced. */
    .card-top {
      display: flex; align-items: center; gap: 0.4rem; width: 100%;
      background: none; border: 0; padding: 0; margin: 0; color: inherit;
      font: inherit; text-align: left; cursor: pointer;
      min-height: var(--control-hit-min);
    }
    /* A team with nothing to open onto is not a toggle: full-strength text (it
       is still the readout), but no pointer promising a tap will do something. */
    .card-top:disabled { cursor: default; }
    .caret { flex-shrink: 0; width: 0; height: 0; border-left: 0.34rem solid var(--ink-dim); border-top: 0.26rem solid transparent; border-bottom: 0.26rem solid transparent; transition: transform 0.1s linear; }
    .card.selected .caret { transform: rotate(90deg); border-left-color: var(--cyan); }
    .team-label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); font-weight: 600; letter-spacing: 0.15em; }
    .status-badge { flex-shrink: 0; font-size: var(--text-xs); padding: 0.05rem 0.3rem; letter-spacing: 0.15em; border: 1px solid; }
    .status-badge.idle { color: var(--ink-dim); border-color: var(--ink-dim); }
    .status-badge.travelling { color: var(--reloading); border-color: var(--reloading); }
    .status-badge.repairing { color: var(--loaded); border-color: var(--loaded); }
    .status-badge.returning { color: var(--cyan); border-color: var(--cyan); }
    /* Off the ship entirely (issue #1386) — its own colour, because "away
       helping someone else" is not a rung on the internal travel cycle. */
    .status-badge.abroad { color: var(--science); border-color: var(--science); }
    .target-label { font-size: var(--text-xs); color: var(--ink-dim); }
    .progress-wrap { width: 100%; height: 0.4rem; background: var(--bg-deep); border: 1px solid var(--line-faint); overflow: hidden; }
    .progress-fill { height: 100%; background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); transition: none; }
    .progress-fill.repairing { background: linear-gradient(90deg, var(--loaded-dim), var(--loaded)); }
    .progress-fill.travelling { background: linear-gradient(90deg, var(--reloading-dim), var(--reloading)); }
    .progress-fill.returning { background: linear-gradient(90deg, var(--cyan-dim), var(--cyan)); }
    .progress-fill.abroad { background: linear-gradient(90deg, var(--cyan-dim), var(--science)); }
    /* The open half of a selected card. Hidden — not merely empty — whenever
       the selected team has no verb to offer, so a returning team's card does
       not open onto a blank box. */
    .card-body { display: flex; flex-direction: column; gap: 0.3rem; border-top: 1px solid var(--line-faint); padding-top: 0.35rem; }
    .card-body[hidden] { display: none; }
    .body-head { font-size: var(--text-xs); letter-spacing: 0.2em; color: var(--ink-dim); text-transform: uppercase; }
    .body-note { font-size: var(--text-xs); color: var(--ink-dim); }
    .dispatch-row { display: flex; flex-wrap: wrap; gap: 0.3rem; }
    /* RECALL sits on its own row so it is never mistaken for one more
       destination, and stretches the card's full width because it is the only
       verb a team already out on a job has. */
    .recall-row { display: flex; }
    .recall-row[hidden] { display: none; }
    .recall-row .btn { flex: 1; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    /* On-site systems (issue #1015's list, re-homed by #1384). Rows are
       buttons: tapping one asks the host to make that system the next job of
       whichever team is already sweeping its station. */
    .damaged-list { display: flex; flex-direction: column; gap: 0.15rem; }
    .dmg-row {
      display: flex; align-items: center; gap: 0.4rem; width: 100%;
      background: var(--bg-card); border: 1px solid var(--line-faint);
      color: inherit; font: inherit; text-align: left; cursor: pointer;
      padding: 0.25rem 0.35rem; min-height: var(--control-hit-min);
    }
    .dmg-row:disabled { cursor: default; opacity: 0.5; }
    .dmg-row.prioritised { border-color: var(--cyan); }
    .dmg-row .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); }
    .dmg-row .tier-chip { font-size: var(--text-xs); letter-spacing: 0.15em; border: 1px solid; padding: 0.02rem 0.25rem; flex-shrink: 0; }
    .dmg-row .tier-chip.damaged { color: var(--reloading); border-color: var(--reloading); }
    .dmg-row .tier-chip.disabled { color: var(--fire); border-color: var(--fire); }
    .dmg-row .tier-chip.destroyed { color: var(--fire); border-color: var(--fire); background: var(--fire-dim); }
    .dmg-row .pct { font-size: var(--text-xs); color: var(--ink-dim); min-width: 2.4rem; text-align: right; flex-shrink: 0; }
    .dmg-row .flag { font-size: var(--text-xs); letter-spacing: 0.15em; color: var(--cyan); flex-shrink: 0; }
  </style>
  <div class="header">
    <span>${t('component.repair_teams.title')}</span>
    <span class="auto-badge" id="auto-badge" style="display:none">${t('console.common.auto')}</span>
  </div>
  <div id="teams-container"></div>
`;
  }

  connectedCallback() {
    super.connectedCallback();
  }

  disconnectedCallback() {
    if (this.#animFrame) {
      cancelAnimationFrame(this.#animFrame);
      this.#animFrame = null;
    }
  }

  set state(val) {
    this.#state = val;
    this.#render();
    if (val) this.#startAnimLoop();
  }

  get state() { return this.#state; }

  /**
   * Which team the player last selected, or `null`. Presentation only. A
   * selected team's card is only OPEN while that team has something to show;
   * a selection made while it did survives a push that empties it.
   */
  get selectedTeamId() { return this.#selectedTeamId; }

  /**
   * Which slot the ship's one field-repair claim holds, or `null` (issue
   * #1386). Authoritative: the host names it on the repair blackboard, and the
   * console no longer reconstructs it by truncating the idle list.
   */
  static abroadTeamId(external) {
    return external && Number.isInteger(external.team_idx) ? external.team_idx : null;
  }

  /**
   * Whether this team is the one abroad.
   *
   * The wire status is checked as well as the index, for the same reason the
   * host's own `is_committed_to_operation` checks the slot: a claim naming a
   * team that has since been given an internal job would otherwise paint a card
   * ABROAD while its team walks across this hull.
   */
  static isAbroad(team, external) {
    return !!team
      && (team.status || 'idle') === 'idle'
      && team.id === PhRepairTeams.abroadTeamId(external);
  }

  /**
   * The 0–1 the bar fills to: a team's own travel/repair progress, or — for the
   * team abroad — the TARGET's condition track, which is the work it is
   * actually doing (issue #1386). `null` there means the target carries no
   * condition track at all, so there is nothing honest to draw and the bar stays
   * empty rather than inventing a number.
   */
  static progressFor(team, external) {
    if (PhRepairTeams.isAbroad(team, external)) {
      return external.target_condition != null ? external.target_condition : 0;
    }
    return team && team.progress_pct != null ? team.progress_pct : 0;
  }

  #startAnimLoop() {
    if (this.#animFrame) return;
    const step = () => {
      const s = this.#state || {};
      const teams = Array.isArray(s.teams) ? s.teams : [];
      const external = s.external_dispatch || null;
      let needsUpdate = false;
      teams.forEach(t => {
        const target = PhRepairTeams.progressFor(t, external);
        const current = this.#displayProgress.get(t.id) ?? target;
        if (Math.abs(current - target) > 0.001) {
          const next = current + (target - current) * 0.15;
          this.#displayProgress.set(t.id, next);
          needsUpdate = true;
        } else {
          this.#displayProgress.set(t.id, target);
        }
      });
      if (needsUpdate) {
        const container = this.shadowRoot.getElementById('teams-container');
        teams.forEach(t => {
          const bar = container.querySelector(`[data-team-id="${t.id}"] .progress-fill`);
          if (bar) {
            const disp = this.#displayProgress.get(t.id) ?? 0;
            bar.style.width = Math.round(disp * 100) + '%';
          }
        });
      }
      this.#animFrame = requestAnimationFrame(step);
    };
    this.#animFrame = requestAnimationFrame(step);
  }

  /** Open the named team's card, or close it if it is the open one. */
  #toggleSelection(teamId) {
    this.#selectedTeamId = this.#selectedTeamId === teamId ? null : teamId;
    this.#render();
  }

  /**
   * Render the on-site systems inside one open card.
   *
   * `rows` is `RepairConsolePayload.damaged_systems` — the visible hull rows
   * that are broken, already worst-first, each flagged with the host's own
   * verdict (`prioritised`, `in_progress`, `prioritisable`). Nothing here
   * re-derives any of that: a tap sends the row's `system_id`, and the same
   * host predicate that applies it also decides whether the row is a live
   * control.
   *
   * A row is only rendered as a CONTROL when tapping it could actually do
   * something. Rows a team is already on site at are the exception the host's
   * own candidate rule creates, and they are shown disabled — see the comment
   * at the `noop` flag below.
   */
  #renderOnSiteSystems(list, rows, auto) {
    const live = new Set(rows.map(r => r.system_id));
    Array.from(list.children).forEach(child => {
      if (!live.has(child.dataset.systemId)) child.remove();
    });

    rows.forEach((row, idx) => {
      let el = list.querySelector(`[data-system-id="${row.system_id}"]`);
      if (!el) {
        el = document.createElement('button');
        el.type = 'button';
        el.className = 'dmg-row';
        el.dataset.systemId = row.system_id;
        el.innerHTML = '<span class="name"></span><span class="flag"></span><span class="tier-chip"></span><span class="pct"></span>';
        el.addEventListener('click', () => {
          if (el.disabled) return;
          // System-targeted on purpose: the ordinal the sweep consumes is
          // resolved host-side, because #737 hides most of the candidates.
          activateEngineeringAction(
            this,
            REPAIR_PRIORITY_ACTION_ID,
            { system_id: row.system_id },
            'set_repair_target_priority',
          );
        });
        list.appendChild(el);
      }
      // Keep DOM order in step with the host's worst-first ordering.
      if (list.children[idx] !== el) list.insertBefore(el, list.children[idx] || null);

      const tier = String(row.tier || '').toLowerCase();
      const name = row.display_name || row.system_id;
      // A row a team is already standing on is the one system the host's sweep
      // never offers as a candidate — `sweep_candidates` excludes every on-site
      // system — so a tap on it resolves to nothing at all. Render it (it is
      // real damage, and the [ON IT] flag is the point of showing it) but do not
      // dress it as a control: an enabled button promising "fix this next" that
      // is structurally incapable of doing so is a lie the player pays for in
      // taps. A prioritised row stays live regardless — that highlight IS a
      // choice the host made.
      // The client sees only a projection and cannot infer which on-site team
      // can sweep which station group. `prioritisable` is projected from the
      // owner's exact candidate predicate, so it is the sole enablement fact.
      const noop = row.prioritisable !== true;
      el.className = 'dmg-row' + (row.prioritised ? ' prioritised' : '');
      el.disabled = auto || noop;
      el.querySelector('.name').textContent = name;
      el.querySelector('.tier-chip').className = 'tier-chip ' + tier;
      el.querySelector('.tier-chip').textContent = t('component.repair_teams.tier.' + tier);
      el.querySelector('.pct').textContent = Math.round((row.damage_pct || 0) * 100) + '%';
      const flag = el.querySelector('.flag');
      flag.textContent = row.prioritised
        ? t('component.repair_teams.next')
        : row.in_progress ? t('component.repair_teams.working') : '';
      el.title = row.prioritised
        ? t('component.repair_teams.prioritised_title', { name })
        : row.in_progress
          ? t('component.repair_teams.working_title', { name })
          : noop
            ? t('component.repair_teams.unavailable_title', { name })
            : t('component.repair_teams.prioritise_title', { name });
    });
  }

  /**
   * Build the DISPATCH TO destinations for one open idle card: one button per
   * damageable station/Core, plus the field target on a hull that has one.
   *
   * The field row is one destination among the stations (issue #1386): it
   * sends the same `DispatchRepairTeam` they do, naming THIS card's team and
   * the `external` target, so choosing where to send a team is one decision
   * whichever side of the hull it lands on. It is still never a recall — a card
   * offers destinations, and the way back is the RECALL row below, which now
   * covers the abroad team too.
   *
   * While another team already holds the ship's one claim the row is a disabled
   * readout carrying that refusal's own text, so the crew are told the thing
   * they have to do (recall the other team) rather than watching a live-looking
   * button be refused.
   *
   * @returns {boolean} whether the card has any destination to offer
   */
  #renderDispatchRow(drow, teamId, targets, external, auto) {
    const fieldWorking = !!(external && external.target != null);
    // The host only names the field target once a team is actually working it
    // (`external_dispatch_target_name` is `None` while nobody is abroad), so an
    // idle card names the destination generically rather than inventing one.
    const fieldLabel = external
      ? (external.target_name
        ? t(external.target_name)
        : t('component.repair_teams.field_target'))
      : null;

    const sig = [
      targets.map(x => x.id).join('|'),
      fieldLabel == null ? '' : fieldLabel,
      fieldWorking ? '1' : '0',
    ].join('#');
    if (drow.dataset.sig !== sig) {
      drow.innerHTML = '';
      targets.forEach(target => {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'btn armed';
        b.dataset.target = target.id;
        b.innerHTML = '<span class="btn-bg"></span><span class="led on"></span><span class="label"></span>';
        b.querySelector('.label').textContent = target.label;
        b.addEventListener('click', () => {
          if (b.disabled) return;
          // target is a station id (lowercase) or 'core'; action-map wraps
          // it into RepairTarget::{Station|Core}.
          activateEngineeringAction(
            this,
            REPAIR_DISPATCH_ACTION_ID,
            { team_idx: teamId, target: target.id },
            'dispatch_repair_team',
          );
        });
        drow.appendChild(b);
      });
      if (fieldLabel != null) {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'btn armed field-btn';
        b.dataset.field = '1';
        b.innerHTML = '<span class="btn-bg"></span><span class="led on"></span><span class="label"></span>';
        b.querySelector('.label').textContent = fieldLabel;
        b.addEventListener('click', () => {
          if (b.disabled) return;
          // The same verb the station buttons beside it send (issue #1386):
          // this card's team, and `external` for the destination. Only the
          // DESTINATION is the host's to resolve — it is whatever Tactical has
          // locked — which is why the target carries no id of its own.
          activateEngineeringAction(
            this,
            REPAIR_DISPATCH_ACTION_ID,
            { team_idx: teamId, target: EXTERNAL_REPAIR_TARGET },
            'dispatch_repair_team',
          );
        });
        drow.appendChild(b);
      }
      drow.dataset.sig = sig;
    }

    drow.querySelectorAll('.btn').forEach(b => {
      b.disabled = auto || (b.dataset.field === '1' && fieldWorking);
    });
    const fieldBtn = drow.querySelector('.field-btn');
    if (fieldBtn) {
      // The host's own refusal id, not a second wording of it: the crew read
      // the same sentence whether they got here by tapping or by reading.
      fieldBtn.title = fieldWorking
        ? t('repair.dispatch.refused.already_abroad')
        : (external && external.refusal ? t(external.refusal) : '');
    }
    return targets.length > 0 || fieldLabel != null;
  }

  /**
   * Build the RECALL control inside one open card whose team is OUT — travelling
   * to a station, working one, or abroad on the field target (issues #1385,
   * #1386).
   *
   * One control, one verb, whatever the team is doing. It names its team, which
   * is what lets it cover both: a ship sends one team abroad at a time, but
   * every internal team can be out on a different job at once, so nothing
   * server-side could resolve which of them "come home" meant.
   *
   * WHETHER a recall is possible is not decided here. The card offers the
   * control off the authoritative status alone — travelling or on site — and
   * the host still owns the rule and answers on the correlated feedback seam.
   * AUTO greys it for the same reason it greys every other verb on this panel:
   * the seat is not the one giving orders.
   */
  #renderRecallRow(row, teamId, auto) {
    let btn = row.querySelector('.recall-btn');
    if (!btn) {
      btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'btn armed recall-btn';
      btn.dataset.recall = String(teamId);
      btn.innerHTML = '<span class="btn-bg"></span><span class="led on"></span><span class="label"></span>';
      btn.querySelector('.label').textContent = t('component.repair_teams.recall');
      btn.addEventListener('click', () => {
        if (btn.disabled) return;
        activateEngineeringAction(
          this,
          REPAIR_RECALL_ACTION_ID,
          { team_idx: teamId },
          'recall_repair_team',
        );
      });
      row.appendChild(btn);
    }
    btn.disabled = auto;
  }

  #render() {
    const s = this.#state || {};
    const teams = Array.isArray(s.teams) ? s.teams : [];
    const auto = !!s.auto;
    const external = s.external_dispatch || null;
    const container = this.shadowRoot.getElementById('teams-container');
    const badge = this.shadowRoot.getElementById('auto-badge');
    badge.style.display = auto ? 'inline' : 'none';

    // Prune before the empty bail-out, so a roster that empties actually
    // empties rather than leaving its last cards behind the empty state.
    const newIds = new Set(teams.map(tm => tm.id));
    Array.from(container.children).forEach(child => {
      if (child !== this.#emptyEl && !newIds.has(Number(child.dataset.teamId))) {
        child.remove();
      }
    });
    // A team that left the roster cannot stay open.
    if (this.#selectedTeamId != null && !newIds.has(this.#selectedTeamId)) {
      this.#selectedTeamId = null;
    }

    if (teams.length === 0) {
      if (!this.#emptyEl) {
        this.#emptyEl = document.createElement('div');
        this.#emptyEl.className = 'empty';
        this.#emptyEl.textContent = t('component.repair_teams.empty');
        container.appendChild(this.#emptyEl);
      }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    // Every station that owns a damageable system gets a destination entry
    // regardless of current damage, so teams can be pre-positioned.
    const targets = Array.isArray(s.targets) ? s.targets : [];
    const damaged = Array.isArray(s.damaged) ? s.damaged : [];

    teams.forEach((team, idx) => {
      let card = container.querySelector(`[data-team-id="${team.id}"]`);
      if (!card) {
        card = document.createElement('div');
        card.className = 'card';
        card.dataset.teamId = team.id;
        // The selector's accessible name is its own content — the team's name
        // and its status badge — so the fallback name is written into the
        // markup here rather than left for the fill below to supply. An
        // element that is a control the moment it is parsed must be named the
        // moment it is parsed; the fill on the next pass only replaces it with
        // whatever the host called this team.
        card.innerHTML = `
          <button type="button" class="card-top" aria-expanded="false">
            <span class="caret" aria-hidden="true"></span>
            <span class="team-label">${t('component.repair_teams.team', { n: team.id })}</span>
            <span class="status-badge">${t('component.repair_teams.status.' + (team.status || 'idle'))}</span>
          </button>
          <span class="target-label"></span>
          <div class="progress-wrap"><div class="progress-fill" style="width:0%"></div></div>
          <div class="card-body" hidden>
            <div class="body-head"></div>
            <div class="body-note"></div>
            <div class="dispatch-row"></div>
            <div class="damaged-list"></div>
            <div class="recall-row" hidden></div>
          </div>
        `;
        card.querySelector('.card-top').addEventListener('click', () => {
          this.#toggleSelection(Number(card.dataset.teamId));
        });
        if (idx < container.children.length) {
          container.insertBefore(card, container.children[idx]);
        } else {
          container.appendChild(card);
        }
      }

      // The wire slot of the team abroad reads `Idle` — it never walked
      // anywhere on this hull — so ABROAD is derived here from the claim the
      // host published (issue #1386) rather than being a fifth `TeamSlot`.
      const isAbroad = PhRepairTeams.isAbroad(team, external);
      const status = isAbroad ? 'abroad' : (team.status || 'idle');
      const isIdle = status === 'idle';
      const selected = this.#selectedTeamId === team.id;
      // Whether this team has a card body AT ALL — settled before any chrome is
      // drawn, because the caret, the selected border and `aria-expanded` are
      // promises about the body and must not outrun it. An idle slot always has
      // one (destinations, or the note saying why there are none). A team out
      // on a job always has one too since issue #1385, because RECALL is a verb
      // it can always use; on site that verb shares the card with the damaged
      // rows the host is showing this seat. Since issue #1386 the team abroad is
      // one of those — one recall verb covers the field claim as well — so the
      // returning team is the only status left with no body at all: the host
      // refuses its recall, and a toggle whose only job is to open an empty box
      // is worse than no toggle.
      const canRecall = status === 'travelling' || status === 'repairing' || isAbroad;
      const hasBody = isIdle || canRecall;
      const open = selected && hasBody;

      const cardTop = card.querySelector('.card-top');
      card.classList.toggle('selected', open);
      cardTop.setAttribute('aria-expanded', open ? 'true' : 'false');
      // A summary with nothing behind it is a readout, not a control: disabled
      // keeps it out of the tab order instead of offering a focus stop that
      // does nothing, and the caret goes invisible — not display:none, so team
      // labels stay aligned down the column — because it points at nothing.
      cardTop.disabled = !hasBody;
      card.querySelector('.caret').style.visibility = hasBody ? '' : 'hidden';
      card.querySelector('.team-label').textContent =
        team.label || t('component.repair_teams.team', { n: team.id });
      const badgeEl = card.querySelector('.status-badge');
      badgeEl.textContent = t('component.repair_teams.status.' + status);
      badgeEl.className = 'status-badge ' + status;

      // The collapsed summary keeps every READOUT — where the team is, or that
      // its slot is spoken for — and offers no verb at all.
      const label = card.querySelector('.target-label');
      if (isAbroad) {
        // The same "Target: …" readout an internal job gets, off the name id
        // the host resolved for the claim. It falls back to the generic field
        // label rather than to an em dash, because there IS a destination — the
        // seat simply has no name for it.
        label.style.display = 'block';
        label.textContent = t('component.repair_teams.target', {
          target: external && external.target_name
            ? t(external.target_name)
            : t('component.repair_teams.field_target'),
        });
      } else if (!isIdle) {
        label.style.display = 'block';
        label.textContent = t('component.repair_teams.target', { target: team.target || '—' });
      } else {
        label.style.display = 'none';
        label.textContent = '';
      }

      const body = card.querySelector('.card-body');
      const head = card.querySelector('.body-head');
      const note = card.querySelector('.body-note');
      const drow = card.querySelector('.dispatch-row');
      const dlist = card.querySelector('.damaged-list');
      const rrow = card.querySelector('.recall-row');
      const clearRow = () => { drow.innerHTML = ''; delete drow.dataset.sig; };
      const clearRecall = () => { rrow.hidden = true; rrow.innerHTML = ''; };

      if (open && isIdle) {
        const hasDestinations = this.#renderDispatchRow(drow, team.id, targets, external, auto);
        head.textContent = t('component.repair_teams.dispatch_to');
        head.style.display = hasDestinations ? 'block' : 'none';
        drow.style.display = hasDestinations ? 'flex' : 'none';
        note.style.display = hasDestinations ? 'none' : 'block';
        note.textContent = hasDestinations ? '' : t('component.repair_teams.no_targets');
        dlist.style.display = 'none';
        dlist.innerHTML = '';
        clearRecall();
      } else if (open) {
        // `hasBody` already established this team is out on an internal job, so
        // it always has RECALL. The damaged rows are the on-site half and are
        // drawn only when the team has actually arrived AND the host is showing
        // this seat something to tap — a travelling team's card is RECALL and
        // nothing else, because #737's information gate has not opened yet.
        clearRow();
        drow.style.display = 'none';
        note.style.display = 'none';
        note.textContent = '';
        const onSiteRows = status === 'repairing' ? damaged : [];
        if (onSiteRows.length > 0) {
          head.textContent = t('component.repair_teams.damaged_title');
          head.style.display = 'block';
          dlist.style.display = 'flex';
          this.#renderOnSiteSystems(dlist, onSiteRows, auto);
        } else {
          head.textContent = '';
          head.style.display = 'none';
          dlist.style.display = 'none';
          dlist.innerHTML = '';
        }
        rrow.hidden = false;
        this.#renderRecallRow(rrow, team.id, auto);
      } else {
        // Closed — either unselected, or a team with no body to open onto (a
        // returning team, a slot committed to the field). Emptied rather than
        // merely hidden, so a closed card holds no control a query could still
        // find.
        clearRow();
        dlist.innerHTML = '';
        note.textContent = '';
        head.textContent = '';
        clearRecall();
      }
      body.hidden = !open;

      const fill = card.querySelector('.progress-fill');
      fill.className = 'progress-fill ' + status;
      const targetPct = PhRepairTeams.progressFor(team, external);
      if (!this.#displayProgress.has(team.id)) {
        this.#displayProgress.set(team.id, targetPct);
      }
      fill.style.width = Math.round(targetPct * 100) + '%';
    });
  }
}

phDefine('ph-repair-teams', PhRepairTeams);
