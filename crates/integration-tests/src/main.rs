#[cfg(test)]
mod kumod;
#[cfg(test)]
use kumod::*;

fn main() {
    println!("Run me via `cargo nextest run` or `cargo test`");
}

#[tokio::test]
async fn w00t() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn(KumoArgs {
        policy_file: "../../simple_policy.lua".to_string(),
        env: vec![],
    })
    .await?;

    println!("Got daemon: {daemon:#?}");

    daemon.stop().await?;

    println!("Stopped!");
    Ok(())
}
