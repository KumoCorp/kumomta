---
description: "Upgrade KumoMTA safely through your package manager, and fix kumo-tsa-daemon startup errors by letting it rebuild tsa.db."
---

# How Do I Upgrade KumoMTA Safely (and Fix tsa.db Errors)?

Upgrading is done through your package manager:

```console
# dnf-based systems
$ sudo dnf update kumomta        # or kumomta-dev

# apt-based systems
$ sudo apt update && sudo apt upgrade kumomta   # or kumomta-dev
```

Note your current version (`kumod --version`) before upgrading, and review the [Changelog](../changelog/index.md) for breaking changes. Upgrade and validate on one node first (`kumod --validate`) before rolling out to the rest. Stable releases target roughly a six-week cadence; if you track `-dev` builds, host the RPM yourself so a build you depend on does not disappear from under you.

## Fixing tsa-daemon errors after an upgrade

The Traffic Shaping Automation database (`tsa.db`) schema can change between releases. After an upgrade you may see `kumo-tsa-daemon` fail to start with an error such as a hash incompatibility or `UNIQUE constraint failed`. The automation state in `tsa.db` is regenerated from incoming events, so the fix is to let the daemon rebuild it:

```console
$ sudo systemctl stop kumo-tsa-daemon
$ sudo mv /var/spool/kumomta/tsa.db /var/spool/kumomta/tsa.db.bak   # path per your configure_tsa_db_path
$ sudo systemctl start kumo-tsa-daemon
```

The daemon recreates a fresh database and repopulates it from the event stream.

!!! warning
    Confirm the actual path of your `tsa.db` from your `configure_tsa_db_path` setting before moving it.

## See also

* [Upgrading](../userguide/installation/upgrading.md)
* [configure_tsa_db_path](../reference/tsa/configure_tsa_db_path.md)
* [Changelog](../changelog/index.md)
