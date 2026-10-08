import Provider, { type Configuration } from "oidc-provider";

import { createPersistentAdapter } from "./adapter.js";
import type { SidecarConfig } from "./config.js";
import {
  assertAllowedResource,
  cimdAllowFetch,
  validateCimdClientMetadata,
  validateDynamicClientMetadata,
} from "./policy.js";

const REFRESH_TOKEN_TTL_SECONDS = 30 * 24 * 60 * 60;
const GRANT_TTL_SECONDS = REFRESH_TOKEN_TTL_SECONDS;

export function buildProviderConfiguration(
  config: SidecarConfig,
): Configuration {
  return {
    jwks: config.jwks,
    cookies: { keys: config.cookieKeys },
    responseTypes: ["code"],
    clientAuthMethods: ["none", "private_key_jwt"],
    clientDefaults: {
      token_endpoint_auth_method: "none",
      grant_types: ["authorization_code"],
      response_types: ["code"],
    },
    scopes: ["openid", "devspace"],
    pkce: {
      required: () => true,
    },
    interactions: {
      url: (_ctx, interaction) => `/interaction/${encodeURIComponent(interaction.uid)}`,
    },
    acceptQueryParamAccessTokens: false,
    features: {
      devInteractions: { enabled: false },
      // Resource server introspection is a separate loopback-only handler.
      // Do not expose oidc-provider's public client introspection route.
      introspection: { enabled: false },
      revocation: { enabled: true },
      resourceIndicators: {
        enabled: true,
        getResourceServerInfo: (_ctx, resource) =>
          assertAllowedResource(resource, config.resourceUrl),
        defaultResource: () => config.resourceUrl,
        useGrantedResource: () => true,
      },
      clientIdMetadataDocument: {
        enabled: true,
        ack: "draft-02",
        allowFetch: (_ctx, uri) => cimdAllowFetch(uri),
        allowClient: (_ctx, client) => {
          validateCimdClientMetadata(client.metadata());
          return true;
        },
      },
      registration: {
        enabled: true,
        initialAccessToken: false,
        issueRegistrationAccessToken: false,
      },
    },
    extraClientMetadata: {
      properties: ["urn:lomway:policy-validation"],
      validator: (ctx, key, _value, metadata) => {
        delete metadata[key];
        if (ctx !== undefined) {
          validateDynamicClientMetadata(metadata);
        }
      },
    },
    issueRefreshToken: (_ctx, client) =>
      client.grantTypes?.includes("refresh_token") === true,
    // Owner-authorized long-lived MCP connections must not expire merely
    // because the browser login Session (default 14 days) ages out.
    expiresWithSession: () => false,
    ttl: {
      Grant: GRANT_TTL_SECONDS,
      RefreshToken: REFRESH_TOKEN_TTL_SECONDS,
      Session: 14 * 24 * 60 * 60,
      Interaction: 60 * 60,
    },
    rotateRefreshToken: true,
    findAccount: async (_ctx, accountId) => {
      if (accountId !== "owner") {
        return undefined;
      }

      return {
        accountId: "owner",
        claims: async () => ({ sub: "owner" }),
      };
    },
  };
}

export function createLomwayProvider(config: SidecarConfig): Provider {
  const provider = new Provider(config.issuer, {
    ...buildProviderConfiguration(config),
    adapter: createPersistentAdapter(config.statePath),
  });
  provider.proxy = true;
  return provider;
}
