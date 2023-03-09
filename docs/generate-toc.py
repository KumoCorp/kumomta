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
        output.write(f"{indent}{bullet}[{self.title}]({self.filename})\n")
        for kid in self.children:
            kid.render(output, depth + 1)


class Gen(object):
    """autogenerate an index page from the contents of a directory"""

    def __init__(self, title, dirname, index=None, extract_title=False):
        self.title = title
        self.dirname = dirname
        self.index = index
        self.extract_title = extract_title

    def render(self, output, depth=0):
        print(self.dirname)
        names = sorted(glob.glob(f"{self.dirname}/*.md"))
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


TOC = [
    Page(
        "KumoMTA Documentation",
        "index.md",
        children=[
            Page("Preface and Legal Notices", "preface/index.md"),
            Page(
                "General Information",
                "general/index.md",
                children=[
                    Page("About This Manual", "general/about.md"),
                    Page("How to Report Bugs or Get Help", "general/report.md"),
                    Page("Credits", "general/credits.md"),
                ],
            ),
            Page(
                "Overview of KumoMTA",
                "overview/index.md",
                children=[
                    Page("KumoMTA History", "overview/history.md"),
                    Page("KumoMTA Architecture", "overview/architecture.md"),
                ],
            ),
            Page(
                "User Guide",
                "guide/index.md",
                children=[
                    Page("Getting Started","guide/getting_started.md",
                        children=[
                            Page("Environmental Considerations","guide/subs/environment_consideration.md"),
                        ]
                    ),
                    Page("Installing for Development","guide/subs/install_for_development.md"),
                    Page("Installing for Production","guide/subs/install_for_production_use.md"),
                    Page("Special instructions for CentOS7 users","guide/subs/special_for_centos7"),
                    Page("Your First Email","guide/subs/your_first_email.md"),
                    Page("Beyond Basics","guide/beyond_basics.md"),
                    Page("Securing It","guide/securing_it.md",
                        children=[
                            Page("Configuring DKIM","guide/subs/dkim.md"),
                            Page("Configuring TLS","guide/subs/tls.md"),
                        ]
                    ),

                    Page("Advanced Configurations","guide/advanced_config.md",
                        children=[
                            Page("Lua Resources","guide/subs/lua_resources.md"),
                            Page("Lua Functions","guide/subs/lua_functions.md"),
                        ]
                    ),
                ]
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
                        "module: kumo.dkim",
                        "reference/kumo.dkim",
                    ),
                    Gen(
                        "module: sqlite",
                        "reference/sqlite",
                    ),
                    Gen(
                        "module: redis",
                        "reference/redis",
                    ),
                    Gen(
                        "object: address",
                        "reference/address",
                    ),
                    Gen(
                        "object: message",
                        "reference/message",
                    ),
                    Gen(
                        "events",
                        "reference/events",
                    ),
                    Gen("HTTP API", "reference/http", extract_title=True),
                ],
            ),
            Page("Change Log", "changelog.md"),
        ],
    )
]

os.chdir("docs")
with open("SUMMARY.md", "w") as f:
    f.write("<!-- this is auto-generated by docs/generate-toc.py, do not edit -->\n")
    for page in TOC:
        page.render(f)
