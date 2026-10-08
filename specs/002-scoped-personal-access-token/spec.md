# Scoped Personal Access Tokens (PAT)

## Purpose

Allow a personally authorized AI client that cannot perform an interactive
OAuth browser login to connect to the existing, OAuth-protected Lomway MCP
endpoint. Keep all existing OAuth clients and refresh-token behavior unchanged.

## Contract and acceptance criteria

1. An operator on the local Windows host can issue a random, named PAT with
   an explicit list of MCP tool names and an expiration of 1–90 days.
2. The plaintext PAT is shown once to the local issuer (via clipboard), never
   stored in the repository, state file, logs, or ChatGPT transcript.
3. Individual PATs can be listed by ID without secret material and revoked.
4. PAT introspection requires exact audience, devspace scope, non-expiration
   and non-revocation. Existing OAuth introspection remains unchanged.
5. PAT requests to tools/list reveal only permitted tools. PAT requests to
   tools/call execute only permitted tool names.
6. The deferred invocation tool lomway_call_tool, if allowed, is permitted
   only when the nested target name is also explicitly permitted.
7. Unknown methods, request batches, malformed bodies, non-POST PAT requests,
   and unrecognized response formats fail closed.
8. Positive/negative regression tests preserve OAuth, MCP tool contracts,
   backend recovery, and no secret leakage.

## Explicit non-goals

- No public PAT issuance endpoint or anonymous registration.
- No unlimited or never-expiring PAT.
- No OAuth login replacement.
- No claim that the remote AI vendor will store a PAT securely.
- No wildcard tool permission or automatic inclusion of future tools.

## Live acceptance

After tests, operator action is needed to issue a PAT, transfer it into the
other AI's secure credential input, and test its MCP initialize/tools/list/
tools/call flow. A private token is not generated in public test output.
