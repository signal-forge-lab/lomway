const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "::1", "[::1]"]);

export function isAllowedOAuthRedirectUri(redirectUri, allowedHosts) {
  let parsed;
  try {
    parsed = new URL(redirectUri);
  } catch {
    return false;
  }

  if (parsed.username || parsed.password || parsed.hash) return false;
  const hostname = parsed.hostname.toLowerCase();
  const protocol = parsed.protocol.toLowerCase();
  if (LOOPBACK_HOSTS.has(hostname)) return protocol === "http:" || protocol === "https:";
  return protocol === "https:" && allowedHosts.some((host) => host.toLowerCase() === hostname);
}

export class AuthorizationAttemptLimiter {
  #attempts = new Map();

  constructor(config) {
    this.config = config;
  }

  retryAfterMs(key, nowMs = Date.now()) {
    const state = this.#attempts.get(key);
    if (!state?.blockedUntilMs) return 0;
    if (state.blockedUntilMs <= nowMs) {
      this.#attempts.delete(key);
      return 0;
    }
    return state.blockedUntilMs - nowMs;
  }

  recordFailure(key, nowMs = Date.now()) {
    const cutoff = nowMs - this.config.failureWindowMs;
    const state = this.#attempts.get(key) ?? { failures: [] };
    state.failures = state.failures.filter((timestamp) => timestamp >= cutoff);
    state.failures.push(nowMs);
    if (state.failures.length >= this.config.maxFailures) {
      state.blockedUntilMs = nowMs + this.config.blockDurationMs;
    }
    this.#attempts.set(key, state);
    return this.retryAfterMs(key, nowMs);
  }

  recordSuccess(key) {
    this.#attempts.delete(key);
  }

  prune(nowMs = Date.now()) {
    const cutoff = nowMs - this.config.failureWindowMs;
    for (const [key, state] of this.#attempts) {
      const failures = state.failures.filter((timestamp) => timestamp >= cutoff);
      const blocked = state.blockedUntilMs !== undefined && state.blockedUntilMs > nowMs;
      if (!blocked && failures.length === 0) this.#attempts.delete(key);
      else this.#attempts.set(key, { failures, blockedUntilMs: blocked ? state.blockedUntilMs : undefined });
    }
  }
}

export function requestAddress(request) {
  return request.ip?.trim() || request.socket?.remoteAddress?.trim() || "unknown";
}

export function delay(milliseconds) {
  if (milliseconds <= 0) return Promise.resolve();
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
