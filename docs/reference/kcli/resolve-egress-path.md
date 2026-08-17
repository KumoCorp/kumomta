# kcli resolve-egress-path


Resolve the effective egress path configuration and throughput ceilings for a destination domain and egress source.

Invokes the same `get_queue_config` and `get_egress_path_config` callbacks that the live runtime would use, performs the associated MX lookup, and reports the resulting configuration, the derived ceilings, and the ready-queue name that would be used. This is the live, server-side counterpart to the `resolve-shaping-domain` script: it operates against a running kumod and so reflects any policy that requires runtime state (e.g. shaping helpers that read from disk at request time).

Default output is a human-readable text block. Pass `--json` for the structured response, or `--config` / `--constraints` to limit to one section.


**Usage:** `kcli resolve-egress-path [OPTIONS] <DOMAIN> [SOURCE]`

## Arguments


* `<DOMAIN>` — The destination domain to resolve

* `<SOURCE>` — The egress source name. Defaults to "unspecified"

## Options


* `--config` — Print only the TOML rendering of the egress path config. Mutually exclusive with --constraints and --json

* `--constraints` — Print only the human-readable ceilings block. Mutually exclusive with --config and --json

* `--json` — Print the full response as pretty JSON. Mutually exclusive with --config and --constraints



