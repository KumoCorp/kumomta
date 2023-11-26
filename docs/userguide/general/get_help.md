#How to Get Help With KumoMTA

Community support for KumoMTA is available both in the forum and in the
community Discord server. Paid support customers should consult their support
SLA document for information on how to contact KumoMTA support and their
relevant guaranteed response and resolution times.

More information about KumoMTA's paid support services can be found at [https://kumomta.com/support](https://kumomta.com/support).

## How To Ask For Help

To get the fastest resolution, start by reading the [Troubleshooting Section](../operation/troubleshooting.md) and following the steps listed there.

If the troubleshooting steps do not help you resolve your issue, please make sure to provide the following when asking for help:

1. The version number of your KumoMTA instance, found using `/opt/kumomta/sbin/kumod --version`

1. The Distro and version of the host Operating System, found using `uname -a`

1. The full text of the init.lua policy script running on the KumoMTA instance.

1. The full text of any and all error messages associated with the issue, with details as to which system generated the error in question (error from injector, error from KumoMTA, error from remote host, etc.)

1. An example of the Swaks call that reproduces the issue. See the [Swaks documentation](http://www.jetmore.org/john/code/swaks/latest/doc/ref.txt) for instructions on how to use Swaks. This allows us to reproduce the issue and removes external factors from the issue at hand.

1. A trace of the communications in question gathered using the `kcli trace-smtp-server` command.

1. Relevant log lines from the KumoMTA logs.

## Discord

The KumoMTA Discord server is intended for real-time communication about
KumoMTA and MailOps/Deliverability in general. The Discord server can be found at
[https://kumomta.com/discord](https://kumomta.com/discord). Please use the #get-help channel to post your questions and be sure to include information on what version of KumoMTA you are using, your configuration, and full error messages.

## Forum

The KumoMTA forum is best suited to longer-form help requests that benefit from
higher visibility in the community (as the help request won't get lost in the
chat scroll or otherwise missed), and which would be of benefit to other users
thanks to the more permanent nature of forum interactions. The forum can be
accessed at [https://forum.kumomta.com](https://forum.kumomta.com).