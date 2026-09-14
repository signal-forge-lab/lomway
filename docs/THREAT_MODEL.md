# Lomway Threat Model

Status: ACTIVE (LMG-G8-01)
Updated: 2026-09-14

This document is the formal threat model for public Lomway v1. It covers the two
structural security properties of the gateway: **loopback isolation** and the
**unauthenticated local boundary**. Every threat is mapped to at least one
enforced mitigation with a concrete enforcement point and regression evidence.

Companion documents: `SECURITY.md` (operational security summary),
`ARCHITECTURE.md` (system structure), `PUBLIC_GENERALIZATION_DESIGN.md`
(public v1 design baseline).

## 1. Scope

In scope:

- the `lomway` binary (config parsing, validation, aggregation router, health);
- the northbound HTTP surface exposed on the loopback listener;
- the southbound HTTP connections to configured MCP backends;
- the configuration file as an input that can widen or narrow the security posture.

Out of scope (owned elsewhere, see §9):

- backend process security and lifecycle (external supervisor);
- remote ingress authentication (OpenAI Secure MCP Tunnel integration);
- secret storage (external SOPS store);
- the operating system, firewall, and other local software;
- client-side security of ChatGPT or any MCP client.

## 2. Assets

| ID | Asset | Sensitivity |
|---|---|---|
| A-01 | Loopback MCP endpoint (`/mcp`) | Arbitrary tool execution against configured backends |
| A-02 | Backend tool catalog and schemas | Exposure of what tools exist; must not be rewritten |
| A-03 | Configuration file | Defines reachable endpoints and policy limits |
| A-04 | Gateway logs | Must not contain secret values |
| A-05 | Gateway availability | Local single-tenant service; no external SLA |

## 3. Trust boundaries and actors

```text
                        UNTRUSTED
  MCP client (ChatGPT or any local MCP client)
      |  (authentication owned by Secure MCP Tunnel integration,
      |   never by the gateway)
      v
====== loopback listener 127.0.0.1:<port> =========================
      |  UNAUTHENTICATED local boundary (accepted, see T-03)
      v
  Lomway gateway (this repository)
      |  (loopback HTTP, no credentials, exact /mcp path)
      v
====== loopback ==================================================
      v
  Configured MCP backends (trusted local services)
```

- **B-1 Northbound:** any process on the machine can open a TCP connection to
  the listener. The gateway performs **no authentication** on this boundary.
- **B-2 Southbound:** the gateway performs **no authentication** toward
  backends and forwards no credentials.
- **B-3 Configuration:** the human/operator who can write the config file is
  trusted to select backends; the config cannot, however, widen the listener
  or backend network scope (see T-01/T-02/T-09).

## 4. Assumptions

1. The machine itself is not attacker-controlled (no hostile local
   administrator or kernel-level attacker is in scope).
2. Backends configured in the file are trusted local services chosen by the
   operator.
3. Remote access, when needed, arrives only through an integration that
   terminates authentication before the gateway (e.g. OpenAI Secure MCP Tunnel).
4. Loopback in this document means exactly `127.0.0.1` (IPv4). Link-local,
   LAN, and unspecified addresses are treated as non-loopback.

## 5. Threat-to-mitigation mapping

STRIDE categories: S spoofing, T tampering, R repudiation, I information
disclosure, D denial of service, E elevation of privilege.

| ID | STRIDE | Threat | Mitigations (enforcement point) | Evidence |
|---|---|---|---|---|
| T-01 | I, E | Listener is bound to a non-loopback address (LAN/`0.0.0.0`), making the unauthenticated tool surface remotely reachable. | `server.host` must be exactly `127.0.0.1` (`src/config/validate.rs` `validate_listener`); `policy.allow_non_loopback_listener = true` is rejected outright in this release (`src/config/validate.rs`); default listen host is `127.0.0.1` (`src/config/model.rs` `DEFAULT_LISTEN_HOST`). | `tests/policy.rs` `rejects_non_loopback_listener`; `src/config/validate.rs` `rejects_non_loopback_listener_and_backends` |
| T-02 | I, E | A backend URL points off-machine, turning the gateway into a relay that sends tool arguments to a remote host. | Every backend URL must match exactly `http://127.0.0.1:<port>/mcp` — HTTP only, no nested path, no TLS host variation (`src/config/validate.rs` `is_exact_loopback_mcp_url`); `policy.allow_non_loopback_backends = true` is rejected outright. | `tests/policy.rs` `rejects_non_http_or_non_loopback_backends`; `src/backend/descriptor.rs` unit tests |
| T-03 | S, E, I | Another local process (or another OS user session) reaches the unauthenticated northbound boundary and invokes tools. | **Accepted residual risk, constrained by design:** listener is loopback-only (T-01), so the boundary is machine-local by construction; the HTTP surface is minimized to `POST/GET/DELETE /mcp`, `GET /healthz`, and `GET /readyz` (`src/gateway/router.rs`) — no admin plane, no metrics; no admin token is configured because none is served. Multi-tenant hardening (local auth) is explicitly out of scope for v1 and would require a new security profile. | `tests/proxy.rs` `external_http_surface_exposes_mcp_and_health_but_not_admin`; `src/gateway/policy.rs` northbound-auth rejection; `docs/SECURITY.md` §3 |
| T-04 | S, E | A remote attacker impersonates the legitimate client without authentication. | The gateway is unreachable remotely by construction (T-01); remote ingress must traverse an integration that terminates authentication before the gateway; the gateway rejects configuring northbound MCP auth or credential forwarding itself (`src/gateway/policy.rs` fails on non-empty auth config). | `tests/policy.rs` `rejects_admin_token_when_admin_plane_is_not_served`, `rejects_schema_rewriting_and_auth_forwarding` |
| T-05 | E | Upstream `mcp-proxy` management plane (`proxy` MCP backend with config/registration tools; `/admin/*` routes) is exposed to clients. | Startup removes the upstream `proxy` control-plane backend and **fails closed** if removal cannot be proven (`src/lib.rs`); the project router serves only `/mcp`, `/healthz`, and `/readyz`; top-level `/admin/*` has no route and nested `/mcp/admin/*` is rejected by middleware before upstream dispatch, regardless of `Authorization` headers. | `tests/proxy.rs` `build_removes_upstream_proxy_control_plane_backend`, `external_http_surface_exposes_mcp_and_health_but_not_admin`; `src/lib.rs`, `src/gateway/router.rs` |
| T-06 | T | A mutating tool call is executed more than once (retry, hedge, failover, coalescing, or response caching duplicating a side effect). | The gateway implements **no** retry, hedging, fan-out, failover, or caching; the only per-call wrapper is a timeout; configuration that enables these features fails validation per backend. Automatic retries remain prohibited unless tool-level idempotency is modeled and tested. | `tests/policy.rs` `rejects_retry_hedging_and_cache_per_backend`, `rejects_fanout_failover_and_request_coalescing`; `tests/proxy.rs` `timeout_does_not_retry_a_mutating_tool` |
| T-07 | S | Namespace confusion: a tool name resolves to the wrong backend (prefix collision, ambiguous normalization, or reserved-prefix impersonation such as `proxy_`). | Backend `id` and `prefix` must be unique and lowercase-restricted; the `_` separator is fixed; final tool names are precomputed and collisions **fail fast** at startup with both sources identified; reserved prefixes (`proxy_`, `lomway_`, legacy `lmg_`) are policy-controlled. | `src/namespace/collision.rs`; `tests/policy.rs` `rejects_wrong_namespace_separator`; `tests/proxy.rs` `same_named_backend_tools_are_namespaced_without_collision` |
| T-08 | D | A client floods the gateway with oversized arguments or long-running calls. | Tool argument size is bounded (default 1 MiB, `security.max_argument_size`); each backend has a configured timeout; no fan-out amplification exists because requests are never duplicated (T-06). | `docs/SECURITY.md` §5; config policy validation (`src/gateway/policy.rs`) |
| T-09 | T, E | A tampered configuration widens the security posture (loopback flags, hot reload, unknown keys sneaking past defaults). | `deny_unknown_fields` at top, `[server]`, and backend level; `schema_version` gate; `allow_non_loopback_listener` / `allow_non_loopback_backends` are rejected even when explicitly set; hot reload is disabled — a reload could accept config that never passed this project's stricter policy validation. | `tests/policy.rs` `rejects_hot_reload_until_gateway_policy_is_revalidated_on_reload`; `src/config/model.rs` strict parsing tests; `src/config/validate.rs` |
| T-10 | I | Secrets leak through logs, config files, or the process environment crossing into public artifacts. | The gateway runtime requires no secret; `${VAR}` references are resolved into the process environment at launch only; no credential forwarding to backends; structured logs exclude secret values; the public-release hygiene test blocks private tunnel paths and local build artifacts. | `tests/policy.rs` `public_release_hygiene_excludes_private_tunnel_paths_and_local_build_artifacts`; `src/config/load.rs` env resolution tests |
| T-11 | T, S | Supply-chain compromise: a dependency (or a newer resolution of it) introduces a control plane or behavioral change. | `mcp-proxy` is exact-pinned (`=0.4.3`) with `default-features = false` and only the protocol feature; `tower-mcp` is exact-pinned (`=0.18.2`); `Cargo.lock` is tracked; every resolution change requires config/schema review, control-plane suppression regression, and full test gates. | `Cargo.toml`, `Cargo.lock`; `docs/SECURITY.md` §7 |
| T-12 | S, T | A rogue local process binds a backend's loopback port and impersonates a trusted backend (southbound boundary is unauthenticated). | **Accepted residual risk:** the southbound boundary is loopback-only with fixed exact `/mcp` paths taken from explicit operator configuration; there is no dynamic backend registration (T-05) so impersonation requires config-level access; backend restarts are recovered by reconnection, not by retrying tool calls. | `tests/proxy.rs` backend-restart recovery tests; §4 assumption 2 |
| T-13 | S, D | Session abuse on `/mcp` (expired or foreign session IDs, unexpected methods). | Streamable HTTP semantics are owned by the pinned upstream stack; expired initial sessions are rejected by the proxy client path; unknown routes return 404. | `tests/proxy.rs` `reject_expired_initial_session`, `external_http_surface_exposes_mcp_and_health_but_not_admin` |

## 6. Loopback isolation model

Loopback isolation is enforced **independently at three layers**, so a single
mistake cannot expose the gateway:

1. **Defaults:** `DEFAULT_LISTEN_HOST = "127.0.0.1"`; example configurations
   use loopback hosts and `${VAR}`-injected loopback URLs.
2. **Schema validation:** every backend URL is an exact
   `http://127.0.0.1:<port>/mcp` match; the listener host must equal
   `127.0.0.1` exactly.
3. **Policy validation:** the escape hatches (`allow_non_loopback_listener`,
   `allow_non_loopback_backends`) are rejected even when set explicitly; there
   is no documented or tested path to a non-loopback bind in this release.

Non-loopback operation requires a future explicit security profile **and a
revision of this threat model** (the T-03/T-04 accepted-risk statement would no
longer hold).

## 7. Unauthenticated local boundary rationale

The northbound and southbound boundaries carry no authentication. This is a
deliberate, documented decision, not an omission:

- **Who is trusted:** every local process of every local user. Loopback
  restricts the boundary to the machine; it does not separate users within it.
- **Why no local auth:** the gateway holds no secrets and grants no privilege
  beyond what any local process could reach by talking to a backend directly;
  adding an admin token would create a secret-bearing component without
  closing any boundary (remote parties still cannot reach loopback).
- **What bounds the damage:** surface minimization (`/mcp`, `/healthz`, `/readyz` only),
  control-plane removal (T-05), no credential storage or forwarding (T-10),
  and fail-closed validation (T-09).
- **Residual risk:** a malicious local process can invoke any configured tool
  and observe tool results, exactly as documented in T-03/T-12. Operators who
  need stronger separation must supply it at the OS level (process/user
  isolation) or via a future authenticated profile.

## 8. Non-goals

- multi-tenant or internet-facing serving;
- gateway-side authentication, authorization, or tenant isolation;
- gateway-side encryption of the loopback hops;
- backend process supervision or sandboxing;
- mitigation of hostile local administrators, kernel attackers, or physical access.

## 9. Out-of-scope owners

| Area | Owner |
|---|---|
| Backend process lifecycle and health outside HTTP | External supervisor (e.g. Swibo integration) |
| Remote ingress authentication | Secure MCP Tunnel integration |
| Secret storage and decryption | External SOPS store resolved at launch |
| OS-level user/process isolation | Operating system configuration |

## 10. Review process

This threat model must be revised when any of the following changes:

- the listener bind policy or loopback validation rules;
- the external HTTP surface (`/mcp`, `/healthz`, `/readyz` naming or scope);
- the pinned `mcp-proxy` / `tower-mcp` versions or their feature sets;
- the retry/cache/fan-out prohibition;
- the configuration schema in a way that can alter reachability (hosts, URLs,
  transports, timeouts-beyond-policy).

Each revision must keep §5 complete: every threat ID keeps at least one
enforced mitigation with current evidence, or is explicitly re-triaged with the
accepted residual risk restated.
