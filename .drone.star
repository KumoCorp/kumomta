# vim:ft=python:ts=4:sw=4:et:

def cache_step(container, is_restore):
    name = "restore-build-cache" if is_restore else "save-build-cache"
    rebuild = "false" if is_restore else "true"
    restore = "true" if is_restore else "false"
    return {
        "name": name,
        "image": "meltwater/drone-cache",
        "environment": {
            "AWS_ACCESS_KEY_ID": {
                "from_secret": "S3_ACCESS_KEY_ID",
            },
            "AWS_SECRET_ACCESS_KEY": {
                "from_secret": "S3_SECRET_KEY",
            },
            "AWS_DISABLESSL": "true",
            "S3_ENDPOINT": {
                "from_secret": "S3_ENDPOINT",
            },
        },
        "settings": {
            "bucket": "drone-ci-cache",
            "region": "eu-west-2",
            "path_style": "true",
            "restore": restore,
            "rebuild": rebuild,
            "cache_key": '{{ .Repo.Name }}_{{ checksum "Cargo.lock" }}_{{ arch }}_{{ os }}_' + container,
            "mount": [
                ".drone-cargo",
                "target",
            ],
        },
    }


def main(ctx):
    return {
        "kind": "pipeline",
        "name": "ubuntu:22",
        "type": "docker",
        "steps": [
            {
                "name": "restore-mtime",
                "image": "python:3-bookworm",
                "commands": [
                    "./assets/ci/git-restore-mtime",
                ],
            },
            cache_step("ubuntu:22.04", True),
            {
                "name": "test",
                "image": "ubuntu:22.04",
                "environment": {
                    "CARGO_HOME": ".drone-cargo",
                },
                "commands": [
                    "echo 'debconf debconf/frontend select Noninteractive' | debconf-set-selections",
                    "apt update",
                    "apt install -y curl git",
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                    ". .drone-cargo/env",
                    "./get-deps.sh",
                    "cargo install cargo-nextest --locked",
                    "cargo build --release",
                    "cargo nextest run --release",
                    "./assets/build-deb.sh",
                ],
            },
            {
                "name": "verify-installable",
                "image": "ubuntu:22.04",
                "commands": [
                    "apt-get install ./kumomta*.deb",
                ],
            },
            {
                "name": "upload-package",
                "image": "alpine:3.14",
                "when": {
                    "branch": {
                        "include": [
                            "master",
                            "drone",  # TODO: remove this
                        ],
                    },
                    "event": {
                        "include": [
                            "tag",
                            "push",
                        ],
                        "exclude": [
                            "pull_request",
                        ],
                    },
                },
                "environment": {
                    "TOKEN": {
                        "from_secret": "OPENREPO_API_TOKEN",
                    },
                },
                "commands": [
                    "apk --no-cache add curl",
                    "/bin/sh ./assets/upload-package.sh ubuntu:22.04 kumomta*.deb",
                ],
            },
            cache_step("ubuntu:22.04", False),
        ],
    }
