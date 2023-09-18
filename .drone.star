# vim:ft=python:ts=4:sw=4:et:


def cache_step(container, is_restore):
    name = "restore-build-cache" if is_restore else "save-build-cache"
    rebuild = "false" if is_restore else "true"
    restore = "true" if is_restore else "false"
    key = container + "_{{ arch }}_{{ os }}"
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
        step["when"] = {"branch": {"include": ["main"]}}
    return step


def restore_cache(container):
    return cache_step(container, True)


def save_cache(container):
    return cache_step(container, False)


def should_publish_package():
    return {
        "event": {
            "include": [
                "tag",
                "push",
            ],
        },
    }


def upload_package(container, filename):
    return {
        "name": "upload-package",
        "image": "alpine:3.14",
        "when": should_publish_package(),
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
        "when": should_publish_package(),
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


def cargo_environment(container):
    env = {
        "CARGO_HOME": ".drone-cargo",
        "CARGO_TERM_COLOR": "always",
    }
    # arm64 is qemu-emulated, reduce the concurrency there to make
    # things a bit easier on the host
    if "arm64" in container:
        cores = "4"
        env["CARGO_BUILD_JOBS"] = cores
        env["NEXTEST_TEST_THREADS"] = cores

    return env


CURL_RETRY_ALL_ERRORS = {
    "ubuntu:20.04": False,
    "rockylinux:8": False,
}


def curl_retry(container):
    flags = "--retry 12"
    probe = CURL_RETRY_ALL_ERRORS.get(container)
    if probe == None:
        probe = True
    if probe:
        flags = flags + " --retry-all-errors"
    return flags


def install_rust(container):
    return [
        "curl "
        + curl_retry(container)
        + " --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ". $CARGO_HOME/env",
    ]


def install_nextest(container):
    arch = "linux-arm" if "arm64" in container else "linux"
    return [
        "test -x .drone-cargo/bin/cargo-nextest || curl "
        + curl_retry(container)
        + " -LsSf https://get.nexte.st/latest/"
        + arch
        + " | tar zxf - -C .drone-cargo/bin"
    ]


def install_deps():
    return [
        "./get-deps.sh",
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
        "trigger": default_trigger(),
        "steps": [
            restore_mtime(),
            restore_cache(container),
            {
                "name": "build",
                "image": container,
                "environment": cargo_environment(container),
                "depends_on": [
                    "restore-build-cache",
                    "restore-mtime",
                ],
                "commands": [
                    "dnf install -y git",
                    # Some systems have curl-minimal which won't tolerate us installing curl
                    "command -v curl || dnf install -y curl",
                ]
                + install_rust(container)
                + install_deps()
                + install_nextest(container)
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


def default_trigger():
    return {
        "event": {
            "exclude": ["promote"],
        },
        # This relies on having
        # https://github.com/meltwater/drone-convert-pathschanged
        # deployed as a conversion plugin in the drone configuration
        "paths": {
            "include": [
                "**/*.rs",
                "**/*.lua",
                "assets/**/*.toml",
                "get-deps.sh",
                "assets/upload-package.sh",
                "assets/build*.sh",
                "assets/install.sh",
                "assets/*.service",
                "**/Cargo.toml",
                ".drone.star",
            ]
        },
    }


def ubuntu(ctx, container):
    arch = "arm64" if "arm64" in container else "amd64"
    pipeline = {
        "kind": "pipeline",
        "name": container,
        "type": "docker",
        "platform": {
            "os": "linux",
            "arch": arch,
        },
        "trigger": default_trigger(),
        "steps": [
            restore_mtime(),
            restore_cache(container),
            {
                "name": "build",
                "image": container,
                "environment": cargo_environment(container),
                "depends_on": [
                    "restore-build-cache",
                    "restore-mtime",
                ],
                "commands": [
                    "echo 'debconf debconf/frontend select Noninteractive' | debconf-set-selections",
                    "apt update",
                    "apt install -y curl git",
                ]
                + install_rust(container)
                + install_deps()
                + install_nextest(container)
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

    if container == "ubuntu:22.04":
        tags = ["latest"]
        if ctx.build.event == "tag":
            tags += [tag_name_from_ref(ctx.build.ref)]
            name = "kumomta"
        else:
            name = "kumomta-dev"

        pipeline["steps"] += [
            {
                "name": "docker-image",
                "image": "plugins/docker",
                "depends_on": ["verify-installable"],
                "settings": {
                    "registry": "ghcr.io",
                    "repo": "ghcr.io/kumocorp/" + name,
                    "username": {
                        "from_secret": "GH_PACKAGE_PUBLISH_USER",
                    },
                    "password": {
                        "from_secret": "GH_PACKAGE_PUBLISH_TOKEN",
                    },
                    "tags": tags,
                    "dockerfile": "docker/kumod/Dockerfile.incremental",
                },
            },
        ]

    return pipeline


def tag_name_from_ref(ref):
    # "refs/tags/something" -> "something"
    return ref[10:]


def main(ctx):
    return [
        # Drone tends to schedule these in the order specified, so
        # let's have a mix of rocky and ubuntu to start, then
        # let the rest get picked up by runners as they become ready
        rocky("rockylinux:9"),
        ubuntu(ctx, "ubuntu:22.04"),
        # ubuntu(ctx, "arm64v8/ubuntu:22.04"),
        ubuntu(ctx, "ubuntu:20.04"),
        rocky("rockylinux:8"),
    ]
