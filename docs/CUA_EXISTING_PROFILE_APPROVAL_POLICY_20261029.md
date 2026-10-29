# Windows Cua existing-profile approval: Lomway operating policy

The Windows Cua backend is normally used for ordinary native GUI tasks
in standard mode, with no existing-browser-profile grant.

For an operation requiring access to the existing authenticated Edge
profile, **prefer BrowserSkill**. If Cua remains necessary, find and
read skill:cua-approval-lease via lomway_search_tools and
lomway_describe_tool before taking action. Obtain explicit user
approval for the exact browser PID/window, purpose, and duration.
Only then use Invoke-CuaLease.ps1 to launch a short-lived Cua
instance on its own private socket. Stop it when done. The lease
expires automatically.

The Windows Cua MCP bridge provides a hard deny for ordinary
browser_prepare calls on the cua-windows backend and advertises this
Skill in its tool description. Ordinary native window tools are
forwarded unchanged. The bridge starts without --grant existing-profile.
This is a guard at the configured Lomway bridge boundary, not a global
OS policy. A process able to run arbitrary Windows commands remains
outside this enforcement. The lease helper's -Approved flag is an
operator attestation, not independently verified user confirmation.

The deployment-specific config/proxy.direct.local.toml is ignored by
Git. Its server instructions include this guidance. Reapply it if
the local config is regenerated. This document is the versioned record.

This policy does not authorize bypassing Cua browser identity checks
or escalating its global permission mode.
