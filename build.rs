use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=frontend/index.html");
    println!("cargo:rerun-if-changed=frontend/vite.config.ts");

    // Check if bun is available, fall back to npm
    let (cmd, args) = if Command::new("bun").arg("--version").output().is_ok() {
        ("bun", vec!["--cwd", "frontend", "run", "build"])
    } else {
        ("npm", vec!["--prefix", "frontend", "run", "build"])
    };

    let status = Command::new(cmd)
        .args(&args)
        .status()
        .unwrap_or_else(|e| panic!("Failed to run {cmd}: {e}"));

    assert!(status.success(), "Frontend build failed");
}
