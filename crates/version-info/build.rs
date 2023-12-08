fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // If a file named `.tag` is present, we'll take its contents for the
    // version number that we report in kumod -V.
    let mut ci_tag = "UNKNOWN-missing-.git-and-.tag-during-build".to_string();

    if let Ok(tag) = std::fs::read("../../.tag") {
        if let Ok(s) = String::from_utf8(tag) {
            ci_tag = s.trim().to_string();
            println!("cargo:rerun-if-changed=../../.tag");
        }
    } else {
        // Otherwise we'll derive it from the git information
        if let Ok(repo) = git2::Repository::discover(".") {
            if let Ok(ref_head) = repo.find_reference("HEAD") {
                let repo_path = repo.path().to_path_buf();

                if let Ok(resolved) = ref_head.resolve() {
                    if let Some(name) = resolved.name() {
                        let path = repo_path.join(name);
                        if path.exists() {
                            println!(
                                "cargo:rerun-if-changed={}",
                                path.canonicalize().unwrap().display()
                            );
                        }
                    }
                }
            }

            if let Ok(output) = std::process::Command::new("git")
                .args(&[
                    "-c",
                    "core.abbrev=8",
                    "show",
                    "-s",
                    "--format=%cd-%h",
                    "--date=format:%Y.%m.%d",
                ])
                .output()
            {
                let info = String::from_utf8_lossy(&output.stdout);
                ci_tag = info.trim().to_string();
            }
        }
    }

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=KUMO_TARGET_TRIPLE={}", target);
    println!("cargo:rustc-env=KUMO_CI_TAG={}", ci_tag);
}
