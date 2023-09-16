# vim:ft=python:ts=4:sw=4:et:


def cache_step(container, is_restore):
    name = "restore-build-cache" if is_restore else "save-build-cache"
    rebuild = "false" if is_restore else "true"
    restore = "true" if is_restore else "false"
    key = 'kumomta_{{ checksum "Cargo.lock" }}_{{ arch }}_{{ os }}_' + container
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
            "cache_key": key,
            "mount": [
                ".drone-cargo",
                "target",
            ],
        },
    }


def restore_cache(container):
    return cache_step(container, True)


def save_cache(container):
    return cache_step(container, False)


def upload_package(container, filename):
    return {
        "name": "upload-package",
        "image": "alpine:3.14",
        "when": {
            "branch": {
                "include": [
                    "master",
                    # "drone",  # TODO: remove this
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
            "apk --no-cache add curl bash",
            "./assets/upload-package.sh " + container + " " + filename,
        ],
    }


def restore_mtime():
    return {
        "name": "restore-mtime",
        "image": "python:3-bookworm",
        "commands": [
            "./assets/ci/git-restore-mtime",
        ],
    }


def rocky(container):
    return {
        "kind": "pipeline",
        "name": container,
        "type": "docker",
        "steps": [
            restore_mtime(),
            restore_cache(container),
            {
                "name": "test",
                "image": container,
                "environment": {
                    "CARGO_HOME": ".drone-cargo",
                },
                "commands": [
                    "dnf install -y git curl",
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                    ". .drone-cargo/env",
                    "./get-deps.sh",
                    "cargo install cargo-nextest --locked",
                    "cargo build --release",
                    "cargo nextest run --release",
                    "./assets/build-rpm.sh",
                    "mv ~/rpmbuild/RPMS/*/*.rpm .",
                ],
            },
            save_cache(container),
            # FIXME: sign rpm
            {
                "name": "verify-installable",
                "image": container,
                "commands": [
                    "dnf install -y ./*.rpm",
                ],
            },
            upload_package(container, "*.rpm"),
        ],
    }


def ubuntu(container):
    return {
        "kind": "pipeline",
        "name": container,
        "type": "docker",
        "steps": [
            restore_mtime(),
            restore_cache(container),
            {
                "name": "test",
                "image": container,
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
            save_cache(container),
            {
                "name": "verify-installable",
                "image": container,
                "commands": [
                    "apt update",
                    "apt-get install -y ./kumomta*.deb",
                ],
            },
            upload_package(container, "kumomta*.deb"),
        ],
    }


def main(ctx):
    return [
        ubuntu("ubuntu:20.04"),
        ubuntu("ubuntu:22.04"),
        rocky("rockylinux:8"),
        rocky("rockylinux:9"),
    ]
