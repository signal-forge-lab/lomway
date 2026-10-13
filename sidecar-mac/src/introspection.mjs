// The macOS sidecar uses the original explicit owner-password OAuth consent.
// It has no per-client tool-grant store. The matching Gateway must therefore
// distinguish its owner-authorized OAuth tokens from restricted PATs.
// Call this ONLY after verifyAccessToken has succeeded; Gateway independently
// validates active, audience, scope, and expiry.
export function ownerAuthorizedIntrospection(info) {
  return {
    active: true,
    client_id: info.clientId,
    scope: info.scopes.join(" "),
    exp: info.expiresAt,
    ...(info.resource ? { aud: info.resource.href } : {}),
    oauth_unrestricted_legacy: true,
  };
}
