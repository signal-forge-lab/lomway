import assert from "node:assert/strict";
import test from "node:test";

import {
  AuthorizationAttemptLimiter,
  isAllowedOAuthRedirectUri,
} from "../src/security.mjs";

test("redirect policy allows ChatGPT HTTPS and loopback only", () => {
  assert.equal(
    isAllowedOAuthRedirectUri("https://chatgpt.com/connector/oauth/callback", ["chatgpt.com"]),
    true,
  );
  assert.equal(isAllowedOAuthRedirectUri("http://127.0.0.1:9911/callback", ["chatgpt.com"]), true);
  assert.equal(isAllowedOAuthRedirectUri("http://chatgpt.com/callback", ["chatgpt.com"]), false);
  assert.equal(isAllowedOAuthRedirectUri("https://evil.example/callback", ["chatgpt.com"]), false);
});

test("failed authorization attempts become temporarily blocked", () => {
  const limiter = new AuthorizationAttemptLimiter({
    maxFailures: 2,
    failureWindowMs: 10_000,
    blockDurationMs: 30_000,
    failureDelayMs: 0,
  });
  assert.equal(limiter.recordFailure("client", 1_000), 0);
  assert.equal(limiter.recordFailure("client", 2_000), 30_000);
  assert.equal(limiter.retryAfterMs("client", 3_000), 29_000);
  limiter.recordSuccess("client");
  assert.equal(limiter.retryAfterMs("client", 3_000), 0);
});
