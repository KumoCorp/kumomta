# Supporting utilities

KumoMTA comes with several utilities that are useful for debugging or supporting KumoMTA.  These are located in `/opt/kumomta/sbin/`.

# Utilities list

* validate-shaping - Used for validating the syntax of your custom shaping files. Using this tool is as simple as providing the shaping file to the utility on the command line. IE: `/opt/kumomta/sbin/validate-shaping /opt/kumomta/etc/policy/shaping.toml`
* resolve-shaping-domain - Used for identifying which traffic shaping rules will be applied to a given destination domain. IE: `/opt/kumomta/sbin/resolve-shaping-domain mosaicco.com`
* tsa-daemon - The TSA Daemon is a tool that can provide centralized traffic shaping data for your entire cluster even across data centers, providing the KumoMTA nodes can connect to it over TCP. This is typically launched from KumoMTA directives as documented [here](../configuration/trafficshaping.md#configure-the-tsa_initlua-file)
* traffic-gen - TrafficGen is a handy performance testing tool that uses core KumoMTA speed to generate high-volume injection testing SMTP messages. Usage instructions are available with `/opt/kumomta/sbin/traffic-gen --help`
* tailer - Tailer provides a flexible command line tool for tracing log activity in real-time without having to `tail -f` the actual logs. It allows you to filter for specific patterns or evaluate a specific batch size of log lines. Usage instructions are available with `/opt/kumomta/sbin/tailer --help`  More details can be found [here](./logs.md#using-tailer).
* proxy-server - KumoProxy is a functional socks5 proxy server that can run independently from KumoMTA.  Usage instructions are available with `/opt/kumomta/sbin/proxy-server --help`
* kcli - KumoMTA Command Line Interface (KCLI) is a useful tool for accessing the HTTP API directly from the command line. Usage instructions are available with `/opt/kumomta/sbin/kcli --help`  More details can be found [here](./kcli.md).
* kumod - this is the actual KumoMTA daemon and is just listed here for completeness.
