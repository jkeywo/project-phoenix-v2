/**
 * Same-origin serialization point for browser save identities when Web Locks
 * are unavailable (#865).
 *
 * A SharedWorker is a browser lifecycle primitive, not a lease: one worker
 * receives every connected document's messages in a single event loop and
 * never decides that a quiet/suspended owner has expired. If a document dies
 * without releasing, that identity stays unavailable (fail closed) until the
 * worker itself ends after the last owning document closes.
 */

const TOKEN_PATTERN = /^[0-9a-f]{32}$/;

function valid(value) {
  return typeof value === 'string' && TOKEN_PATTERN.test(value);
}

export class BrowserSaveIdentityCoordinator {
  constructor() {
    this.owners = new Map();
    this.reservations = new Map();
    this.pendingClaims = new Map();
  }

  connect(port) {
    port.onmessage = (event) => this.receive(port, event && event.data);
    if (typeof port.start === 'function') port.start();
  }

  reply(port, requestId, ok, extra = {}) {
    port.postMessage({ requestId, ok, ...extra });
  }

  availableCandidate(candidates) {
    const reserved = new Set(this.reservations.values());
    return Array.from(candidates || []).find((candidate) => valid(candidate)
      && !this.owners.has(candidate)
      && !reserved.has(candidate)) || null;
  }

  ownAndReply(port, requestId, identity) {
    // One document connection may own only one namespace. This also closes a
    // late-reply hole if a caller ever retries a claim over the same port.
    if (Array.from(this.owners.values()).includes(port)) {
      this.reply(port, requestId, false);
      return false;
    }
    this.owners.set(identity, port);
    this.reply(port, requestId, true, { identity });
    return true;
  }

  waitForPreferred(port, requestId, preferred, candidates, waitMs) {
    const waiting = this.pendingClaims.get(preferred) || [];
    const claim = { port, requestId, candidates, timer: null };
    claim.timer = setTimeout(() => {
      const queued = this.pendingClaims.get(preferred) || [];
      const index = queued.indexOf(claim);
      if (index >= 0) queued.splice(index, 1);
      if (queued.length === 0) this.pendingClaims.delete(preferred);
      const fallback = this.availableCandidate(candidates);
      if (fallback) this.ownAndReply(port, requestId, fallback);
      else this.reply(port, requestId, false);
    }, waitMs);
    waiting.push(claim);
    this.pendingClaims.set(preferred, waiting);
  }

  grantReleasedPreferred(identity) {
    if (Array.from(this.reservations.values()).includes(identity)) return;
    const waiting = this.pendingClaims.get(identity) || [];
    const claim = waiting.shift();
    if (waiting.length === 0) this.pendingClaims.delete(identity);
    if (!claim) return;
    clearTimeout(claim.timer);
    this.ownAndReply(claim.port, claim.requestId, identity);
  }

  receive(port, message) {
    if (!message || typeof message !== 'object') return;
    const { requestId, type } = message;

    if (type === 'claim') {
      const ticket = valid(message.ticket) ? message.ticket : null;
      let identity = null;
      if (ticket) {
        const reserved = this.reservations.get(ticket);
        if (reserved && !this.owners.has(reserved)) {
          identity = reserved;
          this.reservations.delete(ticket);
        }
      }
      if (!identity) {
        const preferred = valid(message.preferred) ? message.preferred : null;
        const waitMs = Number.isFinite(message.waitMs)
          ? Math.max(0, Math.floor(message.waitMs))
          : 0;
        // An ordinary reload has no explicit Start ticket. Give its copied
        // session identity a short chance to be released by the predecessor
        // document, then choose another candidate. This wait never expires or
        // steals OWNERSHIP: a duplicated or suspended live page remains owner,
        // and the waiter gets a different namespace when its own timer fires.
        if (!ticket && preferred && this.owners.has(preferred) && waitMs > 0) {
          this.waitForPreferred(
            port,
            requestId,
            preferred,
            Array.from(message.candidates || []),
            waitMs,
          );
          return;
        }
        identity = this.availableCandidate(message.candidates);
      }
      if (!identity) {
        this.reply(port, requestId, false);
        return;
      }
      this.ownAndReply(port, requestId, identity);
      return;
    }

    if (type === 'handoff') {
      const identity = valid(message.identity) ? message.identity : null;
      const ticket = valid(message.ticket) ? message.ticket : null;
      if (!identity || !ticket || this.owners.get(identity) !== port) {
        this.reply(port, requestId, false);
        return;
      }
      this.owners.delete(identity);
      this.reservations.set(ticket, identity);
      this.reply(port, requestId, true);
      return;
    }

    if (type === 'release') {
      const identity = valid(message.identity) ? message.identity : null;
      if (identity && this.owners.get(identity) === port) {
        this.owners.delete(identity);
        this.grantReleasedPreferred(identity);
      }
      this.reply(port, requestId, true);
    }
  }
}

if (typeof SharedWorkerGlobalScope !== 'undefined'
  && typeof self !== 'undefined'
  && self instanceof SharedWorkerGlobalScope) {
  const coordinator = new BrowserSaveIdentityCoordinator();
  self.onconnect = (event) => {
    for (const port of Array.from(event.ports || [])) coordinator.connect(port);
  };
}
