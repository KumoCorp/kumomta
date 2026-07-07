# Writing Custom Shaping Files

## Writing Your Own Traffic Shaping Rules

The `/opt/kumomta/share/policy-extras/shaping.toml` file provides a collection of traffic shaping rules provided by the KumoMTA team that are useful for new servers. In addition, a community-maintained set of traffic shaping rules is available at `/opt/kumomta/share/community/shaping.toml`.

The files listed above are maintained within the KumoMTA GitHub repository and are updated with each release, meaning that any local edits to these files will be lost any time the KumoMTA install is updated.

In addition, neither of these files is all-encompassing; you will likely encounter scenarios that require you to implement your own logic, either to address your specific reputation or to reflect specialized knowledge you have gained.

To maintain your own traffic shaping rules, create a separate file with your own traffic shaping rules in either TOML or JSON formats, typically called `/opt/kumomta/etc/policy/custom-shaping.[toml|json]` and pass it as part of the call to set up traffic shaping.
