# ADR 0004: OpenTelemetry upgrade path

## Status

Proposed

## Context

Some Rust services (for example under `platform/gh-app`) predate current OpenTelemetry semantic conventions and may mix older SDK/exporter pairs with Jaeger-specific exporters. The monitoring stack under `monitoring/` targets Prometheus and Grafana.

## Decision

- Standardize on **OTLP** export for traces and metrics to align with Grafana Tempo/Mimir or managed backends, rather than bespoke Jaeger-only wiring.
- Plan a single upgrade PR that bumps `opentelemetry` / `opentelemetry_sdk` / `tracing-opentelemetry` together and validates against a local OTLP collector or Grafana Agent.
- Keep **structured logging** with `tracing` and propagate request or correlation IDs in HTTP/gRPC layers.

## Consequences

- Until the upgrade lands, existing telemetry code may remain on older crate versions; new code should follow the OTLP-oriented direction above.
- Helm chart scrape annotations and service names should stay consistent across services after migration.
- CI checks should include at least one telemetry smoke path once migration work starts to prevent silent observability regressions.
