/// Allow embedding the crate into another project, and taking
/// the version information from that project's own git repo.
/// You would set KUMO_VERSION_GIT_REPO_DIR in the environment
/// to the project directory so that we find its git repo instead
/// of the one containing this crate.
fn find_project_git_repo() -> Result<git2::Repository, git2::Error> {
    if let Ok(target) = std::env::var("KUMO_VERSION_GIT_REPO_DIR") {
        git2::Repository::discover(target)
    } else {
        git2::Repository::discover(".")
    }
}

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
        if let Ok(repo) = find_project_git_repo() {
            let repo_path = repo.path().to_path_buf();
            if let Ok(ref_head) = repo.find_reference("HEAD") {
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
                .args([
                    "-c",
                    "core.abbrev=8",
                    "show",
                    "-s",
                    "--format=%cd-%h",
                    "--date=format:%Y.%m.%d",
                ])
                .current_dir(repo_path)
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
