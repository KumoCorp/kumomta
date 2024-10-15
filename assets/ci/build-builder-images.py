#!/usr/bin/env python3
import subprocess
import sys

# argv[1] is the optional image name to build; if not specified,
# all images are built.

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

if len(sys.argv) > 1:
    IMAGE_NAME = sys.argv[1]
    if IMAGE_NAME not in IMAGES:
        raise Exception(f"invalid image name {IMAGE_NAME}")
    IMAGES = [IMAGE_NAME]

for container in IMAGES:
    dockerfile = subprocess.check_output(
        ["assets/ci/emit-builder-dockerfile.py", container]
    ).decode()
    print(dockerfile)

    tag = f"ghcr.io/kumocorp/builder-for-{container}"

    subprocess.run(
        [
            "docker",
            "build",
            "--progress",
            "plain",
            "--no-cache",
            "--file",
            "-",
            "-t",
            tag,
            ".",
        ],
        input=dockerfile,
        encoding="utf-8",
    )

    print(f"Created {tag}")

    # subprocess.run(["docker", "push", tag])
