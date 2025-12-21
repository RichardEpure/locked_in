use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/styles");

    let profile = std::env::var("PROFILE").unwrap_or_default();

    if profile == "release" {
        let status = Command::new("sass")
            .args([
                "assets/styles/main.scss",
                "assets/styles/main.css",
                "--style=compressed",
                "--no-source-map",
            ])
            .status()
            .expect("failed to run `sass`");

        if !status.success() {
            panic!("Sass compilation failed");
        }
    }
}
