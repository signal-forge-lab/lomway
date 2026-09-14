# ADR-0003: v1はnative full tool catalogを優先する

- Status: Accepted
- Date: 2026-09-13

## Context

多数MCPの集約でtool catalogが大きくなる可能性がある。一方、最初からgeneric search/call gatewayへ潰すと、各toolのJSON Schema、annotation、操作意図がclientから見えにくくなる。

## Decision

v1はbackend toolをnamespace付きでnative公開する。初期設定はfull catalogとする。

catalog size、ChatGPT scan、selection qualityを実測し、問題が確認された場合のみsearch exposureへ移行する。

## Consequences

- 型情報とtool identityを最大限維持できる。
- tool数増加の影響をintegration testで監視する必要がある。
- search modeへの変更は別ADR/security review対象。
