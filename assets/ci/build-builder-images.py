#!/usr/bin/env python3
import subprocess
import sys

# Pass the registry hostname:port as argv[1]
REGISTRY = sys.argv[1]

# This script builds a docker image that helps to speed up running
# the build. It is not required to run kumomta itself.
# The images are based on the list of IMAGES below, but with any
# additional dependencies that are required for building pre-installed.

IMAGES = [
    "ubuntu:20.04",
    "ubuntu:22.04",
    "rockylinux:8",
    "rockylinux:9",
    "amazonlinux:2",
    "amazonlinux:2023",
]

for container in IMAGES:
    dockerfile = f"""
FROM {container}\n
WORKDIR /tmp
COPY ./get-deps.sh .
LABEL org.opencontainers.image.source=https://github.com/KumoCorp/kumomta
LABEL org.opencontainers.image.description="Build environment for CI"
LABEL org.opencontainers.image.licenses="Apache"
"""

    commands = [
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ". $HOME/.cargo/env",
        "/tmp/get-deps.sh",
        "curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C /usr/local/bin",
    ]

    if "ubuntu" in container:
        doc_deps = [
            "libcairo2-dev",
            "libffi-dev",
            "libfreetype6-dev",
            "pip",
        ]
        commands = (
            [
                "apt update",
                "apt install -yqq --no-install-recommends "
                + " ".join(
                    [
                        "ca-certificates",
                        "curl",
                        "git",
                    ]
                    + doc_deps
                ),
            ]
            + commands
            + ["cargo install --locked gelatyx"]
            + [
                "pip3 install --quiet "
                + " ".join(
                    [
                        "black",
                        "cairosvg",
                        "mkdocs-exclude",
                        "mkdocs-git-revision-date-localized-plugin",
                        "mkdocs-macros-plugin",
                        "mkdocs-material",
                        "pillow",
                    ]
                )
            ]
        )

        dockerfile += "ENV DEBIAN_FRONTEND=noninteractive\n"
        dockerfile += "RUN rm -f /etc/apt/apt.conf.d/docker-clean\n"
        dockerfile += (
            "RUN --mount=type=cache,target=/var/cache/apt "
            + " && ".join(commands)
            + "\n"
        )

    if "rocky" in container:
        commands = [
            "dnf install -y git rpm-sign gnupg2",
            # Some systems have curl-minimal which won't tolerate us installing curl
            "command -v curl || dnf install -y curl",
        ] + commands
        dockerfile += "RUN " + " && ".join(commands) + "\n"

    if "amazonlinux" in container:
        if container == "amazonlinux:2":
            gpg = "yum install -y gnupg2"
        else:
            gpg = "yum install -y gnupg2 --allowerasing"
        commands = [
            gpg,
            "yum install -y git rpm-sign",
            # Some systems have curl-minimal which won't tolerate us installing curl
            "command -v curl || yum install -y curl",
        ] + commands
        dockerfile += "RUN " + " && ".join(commands) + "\n"

    print(dockerfile)

    tag = f"{REGISTRY}/kumocorp/builder-for-{container}"

    subprocess.run(
        ["docker", "build", "--file", "-", "-t", tag, "."],
        input=dockerfile,
        encoding="utf-8",
    )

    print(f"Created {tag}")

    subprocess.run(["docker", "push", tag])
