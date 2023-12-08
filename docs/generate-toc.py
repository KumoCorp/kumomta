#!/usr/bin/env python3
import base64
import glob
import json
import os
import re
import subprocess
import sys


class Page(object):
    """A page in the TOC, and its optional children"""

    def __init__(self, title, filename, children=None):
        self.title = title
        self.filename = filename
        self.children = children or []

    def render(self, output, depth=0):
        indent = "  " * depth
        bullet = "- " if depth > 0 else ""
        if depth > 0:
            if len(self.children) == 0:
                output.write(f'{indent}{bullet}"{self.title}": {self.filename}\n')
            else:
                output.write(f'{indent}{bullet}"{self.title}":\n')
                if self.filename:
                    output.write(f'{indent}  {bullet}"{self.title}": {self.filename}\n')
        for kid in self.children:
            kid.render(output, depth + 1)


class Gen(object):
    """autogenerate an index page from the contents of a directory"""

    def __init__(self, title, dirname, index=None, extract_title=False, reverse=False):
        self.title = title
        self.dirname = dirname
        self.index = index
        self.extract_title = extract_title
        self.reverse = reverse

    def render(self, output, depth=0):
        names = sorted(glob.glob(f"{self.dirname}/*.md"), reverse=self.reverse)
        children = []
        for filename in names:
            title = os.path.basename(filename).rsplit(".", 1)[0]
            if title == "index" or title == "_index":
                continue

            if self.extract_title:
                with open(filename, "r") as f:
                    title = f.readline().strip("#").strip()

            children.append(Page(title, filename))

        index_filename = f"{self.dirname}/index.md"
        index_page = Page(self.title, index_filename, children=children)
        index_page.render(output, depth)
        with open(index_filename, "w") as idx:
            if self.index:
                idx.write(self.index)
                idx.write("\n\n")
            else:
                try:
                    with open(f"{self.dirname}/_index.md", "r") as f:
                        idx.write(f.read())
                        idx.write("\n\n")
                except FileNotFoundError:
                    pass
            for page in children:
                idx.write(f"  - [{page.title}]({os.path.basename(page.filename)})\n")


class RustDoc(object):
    """autogenerate an index page from the contents of a directory"""

    def __init__(self, title, dirname):
        self.title = title
        self.dirname = dirname

    def render(self, output, depth=0):
        names = sorted(glob.glob(f"{self.dirname}/*/index.html"))
        children = []
        for filename in names:
            crate_name = os.path.dirname(filename)
            title = os.path.basename(crate_name)

            children.append(Page(title, filename))

        index_filename = f"{self.dirname}/index.md"
        index_page = Page(self.title, index_filename, children=children)
        index_page.render(output, depth)
        with open(index_filename, "w") as idx:
            idx.write(
                """
This section contains automatically generated documentation from
the internal Rust code.  It is included in here to aid those
hacking on the internals.

There are no stability guarantees with this API: it may change
without any regard for backward compatibility.

The following crates are part of the KumoMTA workspace:

"""
            )
            for page in children:
                idx.write(f"  - [{page.title}]({page.title}/index.html)\n")


TOC = [
    Page(
        "Tutorial",
        None,
        children=[
            Page("Quickstart Tutorial", "tutorial/quickstart.md"),
            Page("Server Environment", "tutorial/server_environment.md"),
            Page("System Preparation", "tutorial/system_preparation.md"),
            Page("Installing KumoMTA", "tutorial/installing_kumomta.md"),
            Page("Configuring KumoMTA", "tutorial/configuring_kumomta.md"),
            Page("Starting KumoMTA", "tutorial/starting_kumomta.md"),
            Page("Testing KumoMTA", "tutorial/testing_kumomta.md"),
            Page("Checking Logs", "tutorial/checking_logs.md"),
            Page("Next Steps", "tutorial/next_steps.md"),
        ],
    ),
    Page(
        "User Guide",
        "userguide/index.md",
        children=[
            Page(
                "General",
                None,
                children=[
                    Page("Preface and Legal Notices", "userguide/general/preface.md"),
                    Page("About This Manual", "userguide/general/about.md"),
                    Page("How to Report Bugs", "userguide/general/report.md"),
                    Page("How to Get Help", "userguide/general/get_help.md"),
                    Page("Credits", "userguide/general/credits.md"),
                    Page("History", "userguide/general/history.md"),
                    Page("Architecture", "userguide/general/architecture.md"),
                    Page("Lua Fundamentals", "userguide/general/lua.md"),
                ],
            ),
            Page(
                "Installation",
                None,
                children=[
                    Page(
                        "Installation Overview",
                        "userguide/installation/overview.md",
                    ),
                    Page(
                        "Deployment Architecture",
                        "userguide/installation/deployment.md",
                    ),
                    Page("Server Environment", "userguide/installation/environment.md"),
                    Page("Server Hardware", "userguide/installation/hardware.md"),
                    Page(
                        "Operating System", "userguide/installation/operatingsystem.md"
                    ),
                    Page("System Preparation", "userguide/installation/system_prep.md"),
                    Page("Installing on Linux", "userguide/installation/linux.md"),
                    Page("Upgrading", "userguide/installation/upgrading.md"),
                    Page("Installing on Docker", "userguide/installation/docker.md"),
                    Page("Building from Source", "userguide/installation/source.md"),
                ],
            ),
            Page(
                "Configuration",
                None,
                children=[
                    Page(
                        "Configuration Concepts", "userguide/configuration/concepts.md"
                    ),
                    Page(
                        "Lua Policy Helpers",
                        "userguide/configuration/policy_helpers.md",
                    ),
                    Page("Example Server Policy", "userguide/configuration/example.md"),
                    Page("Configuring Spooling", "userguide/configuration/spool.md"),
                    Page("Configuring Logging", "userguide/configuration/logging.md"),
                    Page(
                        "Configuring SMTP Listeners",
                        "userguide/configuration/smtplisteners.md",
                    ),
                    Page(
                        "Configuring Inbound and Relay Domains",
                        "userguide/configuration/domains.md",
                    ),
                    Page(
                        "Configuring Bounce Classification",
                        "userguide/configuration/bounce.md",
                    ),
                    Page(
                        "Configuring Feedback Loop Processing",
                        "userguide/configuration/fbl.md",
                    ),
                    Page(
                        "Configuring HTTP Listeners",
                        "userguide/configuration/httplisteners.md",
                    ),
                    Page(
                        "Configuring Sending IPs",
                        "userguide/configuration/sendingips.md",
                    ),
                    Page(
                        "Configuring Queue Management",
                        "userguide/configuration/queuemanagement.md",
                    ),
                    Page(
                        "Configuring Traffic Shaping",
                        "userguide/configuration/trafficshaping.md",
                    ),
                    Page(
                        "Configuring Traffic Shaping Automation",
                        "userguide/configuration/trafficshapingautomation.md",
                    ),
                    Page("Configuring DKIM Signing", "userguide/configuration/dkim.md"),
                ],
            ),
            Page(
                "Operation",
                None,
                children=[
                    Page("Starting KumoMTA", "userguide/operation/starting.md"),
                    Page("Getting Server Status", "userguide/operation/status.md"),
                    Page(
                        "Troubleshooting KumoMTA",
                        "userguide/operation/troubleshooting.md",
                    ),
                    Page(
                        "Using the kcli Command-Line Client",
                        "userguide/operation/kcli.md",
                    ),
                    Page(
                        "Injecting Messages using SMTP",
                        "userguide/operation/smtpinjection.md",
                    ),
                    Page(
                        "Injecting Messages using HTTP",
                        "userguide/operation/httpinjection.md",
                    ),
                    Page(
                        "Routing Messages Via Proxy Servers",
                        "userguide/operation/proxy.md",
                    ),
                    Page("Viewing Logs", "userguide/operation/logs.md"),
                    Page("Canceling Queued Messages", "userguide/operation/cancel.md"),
                    Page("Performance Tuning", "userguide/operation/performance.md"),
                ],
            ),
            Page(
                "Policy",
                "userguide/policy/index.md",
                children=[
                    Page(
                        "Checking Inbound SMTP Authentication",
                        "userguide/policy/inbound_auth.md",
                    ),
                    Page(
                        "Delivering Messages Using SMTP Auth",
                        "userguide/operation/outbound_auth.md",
                    ),
                    Page("Custom Destination Routing", "userguide/policy/routing.md"),
                    Page(
                        "Routing Messages via HTTP Request", "userguide/policy/http.md"
                    ),
                    Page("Routing Messages via AMQP", "userguide/policy/amqp.md"),
                    Page(
                        "Storing Secrets in Hashicorp Vault",
                        "userguide/policy/hashicorp_vault.md",
                    ),
                    Page(
                        "Publishing Log Events Via Webhooks",
                        "userguide/operation/webhooks.md",
                    ),
                ],
            ),
            Page(
                "Integrations",
                "userguide/integrations/index.md",
                children=[
                    Page(
                        "Ongage",
                        "userguide/integrations/ongage.md",
                    ),
                    Page(
                        "Prometheus",
                        "userguide/integrations/prometheus.md",
                    ),
                ],
            ),
        ],
    ),
    Page(
        "Reference Manual",
        "reference/index.md",
        children=[
            Page("Queues", "reference/queues.md"),
            Gen(
                "module: kumo",
                "reference/kumo",
            ),
            Gen(
                "module: kumo.amqp",
                "reference/kumo.amqp",
            ),
            Gen(
                "module: kumo.digest",
                "reference/kumo.digest",
            ),
            Gen(
                "module: kumo.dkim",
                "reference/kumo.dkim",
            ),
            Gen(
                "module: kumo.dns",
                "reference/kumo.dns",
            ),
            Gen(
                "module: kumo.encode",
                "reference/kumo.encode",
            ),
            Gen(
                "module: kumo.cidr",
                "reference/kumo.cidr",
            ),
            Gen(
                "module: kumo.domain_map",
                "reference/kumo.domain_map",
            ),
            Gen(
                "module: kumo.http",
                "reference/kumo.http",
            ),
            Gen(
                "module: kumo.regex_set_map",
                "reference/kumo.regex_set_map",
            ),
            Gen(
                "module: kumo.secrets",
                "reference/kumo.secrets",
            ),
            Gen(
                "module: kumo.shaping",
                "reference/kumo.shaping",
            ),
            Gen(
                "module: redis",
                "reference/redis",
            ),
            Gen(
                "module: sqlite",
                "reference/sqlite",
            ),
            Gen(
                "module: tsa",
                "reference/tsa",
            ),
            Gen(
                "object: address",
                "reference/address",
            ),
            Gen(
                "object: addressheader",
                "reference/addressheader",
            ),
            Page(
                "object: authenticationresult",
                "reference/authenticationresult.md",
            ),
            Page(
                "object: connectionmeta",
                "reference/connectionmeta.md",
            ),
            Page(
                "object: keysource",
                "reference/keysource.md",
            ),
            Gen(
                "object: message",
                "reference/message",
            ),
            Gen(
                "events",
                "reference/events",
            ),
            Page("HTTP API Explorer", "reference/rapidoc.md"),
            Gen("HTTP API", "reference/http", extract_title=True),
            RustDoc(
                "Internal Rust API",
                "rustapi",
            ),
        ],
    ),
    Gen("Changelog", "changelog", extract_title=True, reverse=True),
]

os.chdir("docs")

with open("../mkdocs.yml", "w") as f:
    f.write("# this is auto-generated by docs/generate-toc.py, do not edit\n")
    f.write("INHERIT: mkdocs-base.yml\n")
    f.write("nav:\n")
    for page in TOC:
        page.render(f, depth=1)
