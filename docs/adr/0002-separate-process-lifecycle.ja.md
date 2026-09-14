# ADR-0002: Gatewayはbackend process lifecycleを所有しない

- Status: Accepted
- Date: 2026-09-13

## Context

各local MCPは既に個別runtime/supervisionを持つ。Gatewayがspawn/kill/restartまで行うと既存Supervisorと責務が重なる。

## Decision

GatewayはHTTP MCP endpointへ接続するだけとし、backendの起動・停止・restartはSwibo等の既存Supervisorへ残す。

## Consequences

- Gateway restartでbackendを巻き込まない。
- backend固有restart policyを維持できる。
- 起動順はSupervisor側で backend → Gateway → Tunnel とする。
- backendがGateway起動時に未起動だった場合のbehaviorをE2Eで確認する必要がある。
