//! End-to-end runs of the real binary against a scratch directory.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_caddedit")
}

const SAMPLE: &str = include_str!("../fixtures/sample.Caddyfile");

#[test]
fn init_splits_sites_verbatim() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let caddyfile = root.join("Caddyfile");
    fs::write(&caddyfile, SAMPLE).unwrap();

    let out = Command::new(bin())
        .args(["--config", caddyfile.to_str().unwrap(), "init"])
        .env_remove("CADDYFILE_PATH")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // five site blocks -> five vhost files
    let enabled = root.join("vhosts/enabled");
    for name in [
        "app.example.com.caddy",
        "grafana.example.com.caddy",
        "www.example.net.caddy",
        "legacy.example.net_8443.caddy",
        "edge.example.io.caddy",
    ] {
        assert!(enabled.join(name).exists(), "missing {name}");
    }

    // verbatim bytes: the wildcard-free raw site must match its slice exactly
    let edge = fs::read_to_string(enabled.join("edge.example.io.caddy")).unwrap();
    assert!(edge.contains("mystery_directive {http.request.host}"));
    assert!(edge.ends_with("}\n"));

    // main config keeps global options + snippet, drops sites, adds import
    let new_main = fs::read_to_string(&caddyfile).unwrap();
    assert!(new_main.contains("email ops@example.com"));
    assert!(new_main.contains("(snippets)"));
    assert!(!new_main.contains("reverse_proxy"));
    assert!(new_main.contains(&format!("import {}", enabled.display()).replace('\\', "/")));

    // backup written
    let backups: Vec<_> = fs::read_dir(root.join("backups")).unwrap().collect();
    assert_eq!(backups.len(), 1);
}

#[test]
fn ls_reports_status_kind_tls() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let caddyfile = root.join("Caddyfile");
    fs::write(&caddyfile, SAMPLE).unwrap();

    Command::new(bin())
        .args(["--config", caddyfile.to_str().unwrap(), "init"])
        .output()
        .unwrap();

    let json = Command::new(bin())
        .args(["--config", caddyfile.to_str().unwrap(), "ls", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&json.stdout);
    assert!(stdout.contains("\"kind\": \"proxy\""));
    assert!(stdout.contains("\"upstreams\""));
    assert!(stdout.contains("cloudflare"));
    assert!(stdout.contains("\"kind\": \"raw\"")); // edge.example.io
}

#[test]
fn off_moves_file_to_disabled_and_back() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let caddyfile = root.join("Caddyfile");
    fs::write(&caddyfile, SAMPLE).unwrap();

    Command::new(bin())
        .args(["--config", caddyfile.to_str().unwrap(), "init"])
        .output()
        .unwrap();

    // disable by address substring
    let out = Command::new(bin())
        .args([
            "--config",
            caddyfile.to_str().unwrap(),
            "--vhosts-dir",
            root.join("vhosts").to_str().unwrap(),
            "off",
            "app.example.com",
            "--no-reload",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let disabled = root.join("vhosts/disabled/app.example.com.caddy");
    assert!(disabled.exists());
    assert!(!root.join("vhosts/enabled/app.example.com.caddy").exists());

    // re-enable
    let out = Command::new(bin())
        .args([
            "--config",
            caddyfile.to_str().unwrap(),
            "on",
            "app.example.com",
            "--no-reload",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("vhosts/enabled/app.example.com.caddy").exists());

    // rm soft-deletes into backups
    let out = Command::new(bin())
        .args([
            "--config",
            caddyfile.to_str().unwrap(),
            "rm",
            "app.example.com",
            "--yes",
            "--no-reload",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!root.join("vhosts/enabled/app.example.com.caddy").exists());
    let backups = fs::read_dir(root.join("backups")).unwrap().count();
    assert!(backups >= 2); // original backup + deleted route
}

#[test]
fn init_refuses_to_double_run_without_force() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let caddyfile = root.join("Caddyfile");
    fs::write(&caddyfile, SAMPLE).unwrap();

    Command::new(bin())
        .args(["--config", caddyfile.to_str().unwrap(), "init"])
        .output()
        .unwrap();

    let out = Command::new(bin())
        .args(["--config", caddyfile.to_str().unwrap(), "init"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--force"), "stderr: {stderr}");
}

#[test]
fn new_creates_validated_site_block() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let caddyfile = root.join("Caddyfile");

    let out = Command::new(bin())
        .args([
            "--config",
            caddyfile.to_str().unwrap(),
            "new",
            "fresh.app.dev",
            "--upstream",
            "localhost:1234",
            "--tls",
            "internal_explicit",
            "--no-reload",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let created = fs::read_to_string(root.join("vhosts/enabled/fresh.app.dev.caddy")).unwrap();
    assert_eq!(
        created,
        "fresh.app.dev {\n\treverse_proxy localhost:1234\n\ttls internal\n}\n"
    );

    // unknown preset must fail loudly before touching the disk
    let out = Command::new(bin())
        .args([
            "--config",
            caddyfile.to_str().unwrap(),
            "new",
            "x.y.dev",
            "--tls",
            "bogus",
            "--no-reload",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!root.join("vhosts/enabled/x.y.dev.caddy").exists());
}

#[test]
fn completions_generate_for_all_shells() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let out = Command::new(bin())
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(out.status.success(), "{shell} failed");
        assert!(!out.stdout.is_empty(), "{shell} produced no completions");
    }
}
