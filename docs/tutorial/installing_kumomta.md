# Installing KumoMTA

Pre-built repos are available for supported Operating Systems, making installation straightforward:

```bash
sudo dnf -y install dnf-plugins-core
sudo dnf config-manager \
    --add-repo \
    https://openrepo.kumomta.com/files/kumomta-rocky.repo
sudo yum install kumomta
```

This installs the KumoMTA daemon to /opt/kumomta/sbin/kumod

!!!note
    Alternatively you can install the kumomta-dev package in order to take advantage of the latest pre-release features. This is only recommended for testing environments.

KumoMTA is now installed with a basic policy that allows relay from localhost, but it will need a more granular configuration policy for production use.

Proceed to the [Configuring KumoMTA](./configuring_kumomta.md) section for more details.