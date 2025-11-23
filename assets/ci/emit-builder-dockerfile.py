#!/usr/bin/env python3
import os
import sys

IMAGES = [
    "ubuntu:20.04",
    "ubuntu:22.04",
    "rockylinux:8",
    "rockylinux:9",
    "amazonlinux:2",
    "amazonlinux:2023",
]

container = sys.argv[1]
if container not in IMAGES:
    raise Exception(f"invalid image name {container}")


dockerfile = f"""
FROM {container}\n
WORKDIR /tmp
COPY ./get-deps.sh .
COPY ./assets/ci/build-rocksdb.sh .
LABEL org.opencontainers.image.source=https://github.com/KumoCorp/kumomta
LABEL org.opencontainers.image.description="Build environment for CI"
LABEL org.opencontainers.image.licenses="Apache"
"""

NEXTEST = "https://get.nexte.st/latest/linux"
if os.getenv("ARM") == "1":
    NEXTEST = "https://get.nexte.st/latest/linux-arm"

SCCACHE_FEATURES = ""
if container == "amazonlinux:2":
    SCCACHE_FEATURES = "--no-default-features"

# Ensure that the image pre-populates the rust toolchain version, to avoid
# consuming additional time and bandwidth in every CI build.
# This is a poor-man's toml parser.
RUST_VERSION = ""
with open("rust-toolchain.toml") as f:
    for line in f:
        fields = line.split("=")
        if len(fields) == 2:
            k = fields[0].strip()
            if k == "channel":
                RUST_VERSION = fields[1].strip()
                break

commands = [
    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
    ". $HOME/.cargo/env",
    "/tmp/get-deps.sh",
    "PREFIX=/opt/kumomta /tmp/build-rocksdb.sh",
    f"curl -LsSf {NEXTEST} | tar zxf - -C /usr/local/bin",
    "rustup install " + RUST_VERSION,
    "cargo install --locked sccache " + SCCACHE_FEATURES,
    "cargo install --locked xcp",
]

if "ubuntu" in container:
    doc_deps = []
    if "ubuntu:22.04" in container:
        doc_deps += ["podman"]

    commands = (
        [
            "echo 'debconf debconf/frontend select Noninteractive' | debconf-set-selections",
            "apt update",
            "apt install -yqq --no-install-recommends "
            + " ".join(
                [
                    "ca-certificates",
                    "curl",
                    "git",
                    "gpg",
                    "jq",
                    "pip",
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
                ]
            )
        ]
        + [
            "apt remove gcc-9 || true",  # it is broken and aws-lc-rs refuses to build with it
            "curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg",
            'echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null',
            "apt update",
            "apt install -yqq --no-install-recommends gh",
        ]
    )

    dockerfile += "ENV DEBIAN_FRONTEND=noninteractive\n"
    dockerfile += "RUN rm -f /etc/apt/apt.conf.d/docker-clean\n"
    dockerfile += "RUN " + " && ".join(commands) + "\n"

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
