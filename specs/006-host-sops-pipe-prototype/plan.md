# Plan

1. Restrict execution to an explicit synthetic environment.
2. Create a local named pipe with explicit SID-based ACLs.
3. Parse a single short JSON request, restrict to secret presence only,
   sanitize all failure output and close the connection.
4. Test presence/absence, disallowed mutation and production mode rejection
   using disposable synthetic SOPS/age inputs.
5. Preserve current Lomway/DSH/ChatGPT, PAT, age key and SOPS ACLs unchanged.
