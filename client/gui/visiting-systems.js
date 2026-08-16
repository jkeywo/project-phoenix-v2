/**
 * gui/visiting-systems.js — the Nav and Comms surfaces a console shows while
 * the human seek has parked them there (issue #984, pasm decision
 * `console-complexity-human-seeking-systems`).
 *
 * "Comms and navigation always try to be under human control": the server
 * walks the hull's seek order every tick and publishes the winner as
 * `host_station` on the system's own blackboard, the client turns that into
 * `s.systems['comms']` on exactly one console's payload
 * (`withVisitingSystems` in gui/console-state.js), and this module turns THAT
 * into a hero-bar button and a full-frame panel.
 *
 * One module rather than a block copied into four consoles, because the
 * destroyer's seek order names all four of its stations and every one of them
 * must render the same thing the same way. A console opts in with the markup
 * convention below plus one call per frame:
 *
 *   <button class="overlay-toggle" id="comms-toggle" data-overlay="comms-overlay" hidden>
 *   <div class="overlay-panel" id="comms-overlay"> … </div>
 *   renderVisitingSystems(s);
 *
 * Every id it touches is fixed, and deliberately so — a console that spells
 * them differently is a console the seek cannot reach, which is a bug that
 * would only show up with a particular crew seated a particular way.
 *
 * The presence of the VIEW is the presence of the button: this module never
 * asks who holds what. That question is answered once, on the wire.
 */
import { setSoughtToggle } from './console-ui.js';

/**
 * `s.systems[id]` if this console is HOLDING that system this frame, else null.
 *
 * Both halves are load-bearing. `hosted_systems` is the seek's answer, and a
 * console that has lost a system must stop offering it even though the view is
 * still in the payload for other panels to read (the destroyer's Intel panel
 * reads the comms view; Intel does not move when Comms does). The view itself
 * is checked too, because a station can be named as host in the same frame the
 * blackboard behind the view has not arrived yet.
 */
function view(s, id) {
  if (!s || !s.systems || !s.systems[id]) return null;
  const hosted = s.hosted_systems;
  // No list at all: a payload built before this field existed. Fall back to
  // "the view is here, so render it", which is what every console did then.
  if (!Array.isArray(hosted)) return s.systems[id];
  return hosted.indexOf(id) >= 0 ? s.systems[id] : null;
}

function el(doc, id) {
  return doc.getElementById(id);
}

/**
 * Show, hide and feed the visiting Nav/Comms surfaces for one render.
 *
 * @param {object} s  the console payload (flat or system-composed — both carry
 *   visiting systems under `systems`)
 * @param {Document} [doc]
 */
export function renderVisitingSystems(s, doc) {
  const root = doc || (typeof document !== 'undefined' ? document : null);
  if (!root) return;

  const n = view(s, 'navigation');
  setSoughtToggle(el(root, 'nav-toggle'), el(root, 'nav-overlay'), !!n);
  if (n) {
    const map = el(root, 'navigation-map');
    if (map) {
      map.state = {
        blips: n.blips || [],
        regions: n.regions || [],
        range: n.radar_range || 5000,
        ship_pos: { x: n.ship_x || 0, z: n.ship_z || 0 },
        ship_heading: n.ship_heading || 0,
        waypoint: n.waypoint || null,
      };
    }
    // Civilian traffic (issue #1028): who is on which lane, and who is not
    // doing as asked. Server-derived; the panel never infers it.
    const civilians = el(root, 'civilian-traffic');
    if (civilians) civilians.state = { civilians: n.civilians || [] };
  }

  const c = view(s, 'comms');
  setSoughtToggle(el(root, 'comms-toggle'), el(root, 'comms-overlay'), !!c);
  if (c) {
    const contacts = el(root, 'comms-contact-list');
    if (contacts) contacts.state = { contacts: c.contacts || [] };
    const messages = c.messages || [];
    const unread = el(root, 'comms-unread');
    if (unread) {
      unread.classList.toggle('show', messages.some(function (m) { return !m.is_read; }));
    }
    const current = el(root, 'comms-current-message');
    if (current) {
      // The oldest unread is the one being asked about; with nothing unread the
      // panel keeps the last exchange on screen rather than going blank.
      const thread = messages.find(function (m) { return !m.is_read; })
        || messages[messages.length - 1]
        || null;
      current.state = { thread: thread, rejection: c.rejection };
    }
  }
}
