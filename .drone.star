# vim:ft=python:ts=4:sw=4:et:


def cache_step(container, is_restore):
    name = "restore-build-cache" if is_restore else "save-build-cache"
    rebuild = "false" if is_restore else "true"
    restore = "true" if is_restore else "false"
    key = 'kumomta_{{ checksum "Cargo.lock" }}_{{ arch }}_{{ os }}_' + container
    step = {
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
    if not is_restore:
        step["depends_on"] = ["build"]
    return step


def restore_cache(container):
    return cache_step(container, True)


def save_cache(container):
    return cache_step(container, False)


def main_branch_or_tag():
    return {
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
    }


def upload_package(container, filename):
    return {
        "name": "upload-package",
        "image": "alpine:3.14",
        "when": main_branch_or_tag(),
        "depends_on": ["verify-installable"],
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


def sign_rpm(container):
    return {
        "name": "sign-rpm",
        "image": container,
        "when": main_branch_or_tag(),
        "depends_on": ["build"],
        "environment": {
            "PUB": {
                "from_secret": "OPENREPO_GPG_PUBLIC",
            },
            "PRIV": {
                "from_secret": "OPENREPO_GPG_PRIVATE",
            },
        },
        "commands": [
            "dnf install -y rpm-sign gnupg2",
            "./assets/sign-rpm.sh *.rpm",
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


def cargo_environment():
    return {
        "CARGO_HOME": ".drone-cargo",
        "CARGO_TERM_COLOR": "always",
    }


def install_rust():
    return [
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ". $CARGO_HOME/env",
    ]


def install_deps():
    return [
        "./get-deps.sh",
        "cargo install cargo-nextest --locked",
    ]


def perform_build_and_test():
    return [
        "cargo build --release",
        "cargo nextest run --release",
    ]


def rocky(container):
    return {
        "kind": "pipeline",
        "name": container,
        "type": "docker",
        "steps": [
            restore_mtime(),
            restore_cache(container),
            {
                "name": "build",
                "image": container,
                "environment": cargo_environment(),
                "depends_on": [
                    "restore-build-cache",
                    "restore-mtime",
                ],
                "commands": [
                    "dnf install -y git",
                    # Some systems have curl-minimal which won't tolerate us installing curl
                    "command -v curl || dnf install -y curl",
                ]
                + install_rust()
                + install_deps()
                + perform_build_and_test()
                + [
                    "./assets/build-rpm.sh",
                    "mv ~/rpmbuild/RPMS/*/*.rpm .",
                ],
            },
            save_cache(container),
            sign_rpm(container),
            {
                "name": "verify-installable",
                "image": container,
                "depends_on": ["sign-rpm"],
                "commands": [
                    "dnf install -y ./*.rpm",
                ],
            },
            upload_package(container, "*.rpm"),
        ],
    }


def ubuntu(container):
    arch = "arm64" if "arm64" in container else "amd64"
    return {
        "kind": "pipeline",
        "name": container,
        "type": "docker",
        "platform": {
            "os": "linux",
            "arch": arch,
        },
        "steps": [
            restore_mtime(),
            restore_cache(container),
            {
                "name": "build",
                "image": container,
                "environment": cargo_environment(),
                "depends_on": [
                    "restore-build-cache",
                    "restore-mtime",
                ],
                "commands": [
                    "echo 'debconf debconf/frontend select Noninteractive' | debconf-set-selections",
                    "apt update",
                    "apt install -y curl git",
                ]
                + install_rust()
                + install_deps()
                + perform_build_and_test()
                + [
                    "./assets/build-deb.sh",
                ],
            },
            save_cache(container),
            {
                "name": "verify-installable",
                "image": container,
                "depends_on": ["build"],
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
        ubuntu("arm64v8/ubuntu:22.04"),
        rocky("rockylinux:8"),
        rocky("rockylinux:9"),
    ]
