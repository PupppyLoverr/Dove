// Reviews: a consequential action waits here until the user decides in an
// extension window (never inside the page, which could fake a click).

export class Reviews {
  constructor({ timeoutMs = 120_000, now = () => Date.now() } = {}) {
    this.timeoutMs = timeoutMs;
    this.now = now;
    this.pending = new Map();
    this.next = 1;
  }

  /** Queue a review; resolves true (allowed) or false (denied, timed out, stopped). */
  request(details) {
    const id = String(this.next++);
    let resolve;
    const decided = new Promise((r) => { resolve = r; });
    const timer = setTimeout(() => this.decide(id, false, 'timed out'), this.timeoutMs);
    this.pending.set(id, { id, details, at: this.now(), resolve, timer });
    return { id, decided };
  }

  decide(id, allow, why = allow ? 'allowed' : 'denied') {
    const r = this.pending.get(String(id));
    if (!r) return false;
    clearTimeout(r.timer);
    this.pending.delete(String(id));
    r.resolve({ allow: !!allow, why });
    return true;
  }

  /** The user pressed Stop: every waiting review is denied. */
  denyAll(why = 'stopped') {
    for (const id of [...this.pending.keys()]) this.decide(id, false, why);
  }

  list() {
    return [...this.pending.values()].map(({ id, details, at }) => ({ id, details, at }));
  }
}
