import { isIP } from "node:net";
import { errors, type ResourceServer } from "oidc-provider";

import {
  assertPublicCimdHost,
  assertSafeCimdUrl,
  isPublicAddress,
} from "./security.js";

/**
 * Validates that the requested resource matches the configured resource exactly.
 * Returns the audience-bound ResourceServer definition with scope 'devspace'.
 * Throws oidc-provider.errors.InvalidTarget if the target does not match.
 */
export function assertAllowedResource(
  resource: unknown,
  configuredResource: string,
): ResourceServer {
  if (typeof resource !== "string" || resource !== configuredResource) {
    throw new errors.InvalidTarget("requested resource is not allowed");
  }

  return {
    scope: "devspace",
    audience: configuredResource,
    accessTokenTTL: 3600,
  };
}

/**
 * Helper returning exactly the configured resource indicator.
 */
export function defaultResource(configuredResource: string): string {
  return configuredResource;
}

/**
 * Validates dynamic client registration metadata for compatibility with the
 * Lomway deployment: public client only, authorization_code, code response,
 * valid redirect URIs (HTTPS or loopback HTTP).
 */
export function validateDynamicClientMetadata(
  metadata: unknown,
  allowedTokenEndpointAuthMethods: readonly string[] = ["none"],
): void {
  if (typeof metadata !== "object" || metadata === null || Array.isArray(metadata)) {
    throw new errors.InvalidClientMetadata("client metadata must be an object");
  }

  const meta = metadata as Record<string, unknown>;

  if (
    meta.token_endpoint_auth_method !== undefined &&
    (
      typeof meta.token_endpoint_auth_method !== "string" ||
      !allowedTokenEndpointAuthMethods.includes(meta.token_endpoint_auth_method)
    )
  ) {
    throw new errors.InvalidClientMetadata(
      `token_endpoint_auth_method must be omitted or one of: ${allowedTokenEndpointAuthMethods.join(", ")}`,
    );
  }

  if (meta.grant_types !== undefined) {
    const grantTypes = meta.grant_types;
    if (
      !Array.isArray(grantTypes) ||
      new Set(grantTypes).size !== grantTypes.length ||
      !grantTypes.includes("authorization_code") ||
      grantTypes.some(
        (grantType) =>
          grantType !== "authorization_code" && grantType !== "refresh_token",
      )
    ) {
      throw new errors.InvalidClientMetadata(
        "grant_types must contain authorization_code and may also contain refresh_token",
      );
    }
  }

  if (meta.response_types !== undefined) {
    if (
      !Array.isArray(meta.response_types) ||
      meta.response_types.length !== 1 ||
      meta.response_types[0] !== "code"
    ) {
      throw new errors.InvalidClientMetadata(
        "response_types must be omitted or ['code']",
      );
    }
  }

  if (!Array.isArray(meta.redirect_uris) || meta.redirect_uris.length === 0) {
    throw new errors.InvalidClientMetadata("redirect_uris must be a non-empty array");
  }

  for (const rawUri of meta.redirect_uris) {
    if (typeof rawUri !== "string" || rawUri.trim() === "") {
      throw new errors.InvalidClientMetadata("redirect_uri must be a non-empty string");
    }

    let url: URL;
    try {
      url = new URL(rawUri);
    } catch {
      throw new errors.InvalidClientMetadata("redirect_uri must be a valid absolute URL");
    }

    if (url.username !== "" || url.password !== "") {
      throw new errors.InvalidClientMetadata("redirect_uri must not contain credentials");
    }

    if (url.hash !== "") {
      throw new errors.InvalidClientMetadata("redirect_uri must not contain a fragment");
    }

    if (url.protocol === "http:") {
      if (url.hostname !== "127.0.0.1" && url.hostname !== "[::1]") {
        throw new errors.InvalidClientMetadata(
          "http redirect_uri must use loopback address 127.0.0.1 or [::1]",
        );
      }
    } else if (url.protocol === "https:") {
      const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, "");
      if (
        host === "" ||
        host === "localhost" ||
        host.endsWith(".localhost") ||
        host.endsWith(".local") ||
        host.endsWith(".internal") ||
        host.endsWith(".lan") ||
        host.endsWith(".home.arpa") ||
        (isIP(host) === 0 && !host.includes(".")) ||
        (isIP(host) !== 0 && !isPublicAddress(host))
      ) {
        throw new errors.InvalidClientMetadata(
          "https redirect_uri host must not be localhost, internal, or private address",
        );
      }
    } else {
      throw new errors.InvalidClientMetadata(
        "redirect_uri scheme must be https or loopback http",
      );
    }
  }
}

/**
 * Validates a metadata-document-resolved client.
 *
 * ChatGPT currently advertises both "none" and "private_key_jwt" in
 * token_endpoint_auth_methods_supported and publishes the legacy singular
 * token_endpoint_auth_method as its preference during the transition.
 *
 * DCR remains public-client-only; this broader policy is used only by CIMD.
 */
export function validateCimdClientMetadata(metadata: unknown): void {
  validateDynamicClientMetadata(metadata, ["none", "private_key_jwt"]);

  const meta = metadata as Record<string, unknown>;

  if (meta.token_endpoint_auth_methods_supported !== undefined) {
    const methods = meta.token_endpoint_auth_methods_supported;
    if (
      !Array.isArray(methods) ||
      methods.length === 0 ||
      new Set(methods).size !== methods.length ||
      methods.some(
        (method) =>
          method !== "none" &&
          method !== "private_key_jwt",
      )
    ) {
      throw new errors.InvalidClientMetadata(
        "token_endpoint_auth_methods_supported must contain only supported CIMD methods",
      );
    }

    if (
      typeof meta.token_endpoint_auth_method === "string" &&
      !methods.includes(meta.token_endpoint_auth_method)
    ) {
      throw new errors.InvalidClientMetadata(
        "token_endpoint_auth_method must be included in token_endpoint_auth_methods_supported",
      );
    }
  }

  if (meta.token_endpoint_auth_method === "private_key_jwt") {
    if (typeof meta.client_id !== "string") {
      throw new errors.InvalidClientMetadata(
        "private_key_jwt CIMD client must include client_id",
      );
    }
    if (typeof meta.jwks_uri !== "string") {
      throw new errors.InvalidClientMetadata(
        "private_key_jwt CIMD client must include jwks_uri",
      );
    }

    const clientIdUrl = assertSafeCimdUrl(meta.client_id);
    const jwksUrl = assertSafeCimdUrl(meta.jwks_uri);

    if (clientIdUrl.origin !== jwksUrl.origin) {
      throw new errors.InvalidClientMetadata(
        "private_key_jwt CIMD jwks_uri must use the same origin as client_id",
      );
    }
  }
}

/**
 * Runs purely syntactic checks on a CIMD client identifier URL.
 * Returns the parsed URL without performing network or DNS operations.
 */
export function cimdPreflight(clientId: string): URL {
  return assertSafeCimdUrl(clientId);
}

/**
 * Checks whether a CIMD URL is safe to fetch by performing syntactic validation
 * and DNS resolution check. Returns false if any check fails (fail closed).
 */
export async function cimdAllowFetch(clientId: string): Promise<boolean> {
  try {
    const url = assertSafeCimdUrl(clientId);
    await assertPublicCimdHost(url);
    return true;
  } catch {
    return false;
  }
}
