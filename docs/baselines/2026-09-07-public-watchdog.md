# Public deployment recovery — 2026-09-07

## Incident and scope

Local 3005 and public router 3006 returned HTTP 200 while both VPS relay HTTP and HTTPS requests stalled. nginx and both FRP processes remained alive. FRP logs contained work-connection timeouts. Restarting only the tracked client restored public HTTP 200; this locates the failed forwarding layer, not the underlying transport defect.

No application binaries, accounts, provider settings or user data were changed. Flash-only access and the existing worker limit remain unchanged.

## Recovery implementation

- `tools/watch-public.ps1`: direct, certificate-verified public GET `/health` and local GET `/health`, with a six-second request timeout and a 15-second interval. Three consecutive failures trigger diagnosis. An SSH-pinned remote relay probe distinguishes tunnel failure from external HTTPS routing failure. Probes do not log in, create sessions or invoke models.
- `tools/repair-public-tunnel.ps1`: checks PID, executable and creation time before replacing only frpc. Router, workers and local 3005 stay running. State replacement is atomic.
- Public lifecycle scripts share a mutex. Manual stop creates a persistent `disabled` marker. Recovery honors it; explicit manual start clears it. Unknown process identities are not terminated.
- Recovery retries back off from 60 seconds to ten minutes. The guard records sanitized events and a current `watch-status.json` in the existing private deployment directory. Its event log rotates at 1 MiB.
- `tools/install-public-watch.ps1` attempts a same-user scheduled task. This machine denied task registration, so the deployed installation uses the current user's `Run` registry entry and `tools/host-public-watch.ps1`. The host restarts an exited guard after 15 seconds. Named mutexes prevent duplicate hosts/guards.

## Operational boundaries

The installed fallback starts when the deployment Windows account logs in, preserving access to its existing DPAPI credentials and user-scoped environment. Locking the screen is supported. Logout, shutdown or sleep interrupts hosting. Startup before login requires a separately provisioned service identity; this release does not promise that behavior.

The guard's health endpoint verifies the route to the router, not each authenticated worker or the external model provider. It cannot make home-network or cloud outages disappear. It never automatically resubmits a user analysis. Persistent failures stay visible in status/logs with bounded recovery frequency.

Manual operations remain `tools/start-public.ps1` and `tools/stop-public.ps1`; the latter disables automatic recovery. Installation is repeatable with `tools/install-public-watch.ps1`.

## Verification

- All six deployment PowerShell scripts passed syntax parsing.
- Initial tunnel-only repair restored certificate-verified public HTTP 200.
- Controlled tracked-frpc exit: the guard automatically replaced frpc and returned to `healthy` within 62 seconds of fault injection. Public router and local 3005 PIDs remained unchanged.
- Controlled guard exit: the persistent host launched a new guard, which reported `healthy`; the installed user-login startup entry was verified.
- Final public checks: `/health` and `/login.html` returned 200; anonymous `/api/agent/providers` returned the expected 401.
- Full router recovery, Windows reboot/login and a physical network outage are not exercised against live users in this deployment.
