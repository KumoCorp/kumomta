---
description: Test your KumoMTA traffic shaping files with the validate-shaping utility, checking custom shaping.toml override syntax before deploying to production.
---

# Testing Your Shaping Files

Included in the standard deployment is a validation tool for testing the syntax of your shaping.toml override file. The file located at `/opt/kumomta/sbin/validate-shaping` can be used to validate the syntax of your shaping file.  If there are no errors, it will return an "OK".

```bash
$ /opt/kumomta/sbin/validate-shaping /opt/kumomta/etc/policy/custom-shaping.toml
OK
```
