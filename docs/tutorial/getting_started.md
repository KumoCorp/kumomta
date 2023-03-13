# Getting Started with KumoMTA

## What is KumoMTA?

KumoMTA is an open source Message Transfer Agent (MTA) designed to provide high performance outbound email functionality.

KumoMTA is deployable from  RPM, or as a Docker container or you can build it with Rust crates. If you just want to use it to send mail, you can follow the easy path below. Alternately, you can install as a developer/contributor and have full access to the source code of the core MTA.  Contributions from the community are welcome.

If you have no idea what an MTA is then [this may be a good primer](https://en.wikipedia.org/wiki/Message_transfer_agent) before you get too deep into the documentation here.  If you DO know what an MTA is and you are looking for an open source option to support, then read on.

## How do I install it?

=== "CentOS7"

    !!! note
        Note that Red Hat full support for RHEL 7
        [ended in August 2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates)
        and CentOS 7 full support [ended in August 2020](https://wiki.centos.org/About/Product).
        We recommend upgrading to a newer OS as soon as possible.


    ```
    $ sudo yum-config-manager --add-repo=https://raw.githubusercontent.com/kumomta/kumomta/main/assets/yum.repos.d/kumomta-centos7.repo
    $ sudo yum install kumomta-dev
    ```

=== "Rocky"
    ```
    $ sudo do something for rocky
    ```

=== "Ubuntu"
    ```
    $ sudo do something for ubuntu
    ```

If you want to explore KumoMTA in **Docker containers**, potentially orchestrated with Kubernetes, You should follow the instructions [here](install_with_docker.md).

If you want to experiment, contrubute, or hack stuff up, follow the instructions for [**Building from Source**](install_from_source.md).

## What's next?

Read through the environment considerations and system preparation sections to make sure you have a right-sized server, then install the version you need based on your reading above.  Modify your config to make it uniquely yours, then test with a small sample of receivers.



