//! Output-contract tests for the `sap-odata` binary.
//!
//! The CLI is consumed by scripts and AI agents (see
//! `skills/sap-odata-cli/SKILL.md`), so stdout/stderr separation, exit
//! codes and JSON shapes are a public contract. These tests execute the
//! compiled binary against an isolated config directory
//! (`SAP_ODATA_CONFIG_DIR`) and never touch the network or the OS keyring.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Environment variables that could leak the developer's real setup into
/// a test run.
const SCRUBBED_ENV: &[&str] = &[
    "SAP_BASE_URL",
    "SAP_CLIENT",
    "SAP_LANGUAGE",
    "SAP_USER",
    "SAP_PASSWORD",
    "RUST_LOG",
];

/// Minimal V4 EDMX that passes the offline import validation pipeline.
const MINIMAL_V4_EDMX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx Version="4.0" xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx">
  <edmx:DataServices>
    <Schema Namespace="com.sap.gateway.srvd.zcontract_test.v0001" xmlns="http://docs.oasis-open.org/odata/ns/edm">
      <EntityType Name="ItemType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container">
        <EntitySet Name="Item" EntityType="com.sap.gateway.srvd.zcontract_test.v0001.ItemType"/>
      </EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>
"#;

/// An isolated config directory, removed on drop.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sap_odata_cli_contract_{label}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sap-odata"));
        for var in SCRUBBED_ENV {
            cmd.env_remove(var);
        }
        cmd.env("SAP_ODATA_CONFIG_DIR", &self.dir);
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd()
            .args(args)
            .output()
            .expect("failed to spawn sap-odata")
    }

    fn write_file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ── help output ──

#[test]
fn help_never_prints_credential_env_values() {
    let sb = Sandbox::new("help_env");
    let out = sb
        .cmd()
        .env("SAP_USER", "canary-user-7f3a")
        .env("SAP_PASSWORD", "canary-secret-7f3a")
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = stdout(&out);
    // The variable names stay documented …
    assert!(
        help.contains("SAP_PASSWORD"),
        "help should name the env var"
    );
    assert!(help.contains("SAP_USER"), "help should name the env var");
    // … but their values never appear.
    assert!(
        !help.contains("canary-secret-7f3a"),
        "password leaked:\n{help}"
    );
    assert!(
        !help.contains("canary-user-7f3a"),
        "username leaked:\n{help}"
    );
}

// ── config isolation ──

#[test]
fn config_dir_env_override_isolates_the_offline_library() {
    let sb = Sandbox::new("config_env");
    let edmx = sb.write_file("contract.edmx", MINIMAL_V4_EDMX);

    let out = sb.run(&["--json", "offline", "import", edmx.to_str().unwrap()]);
    assert!(out.status.success(), "import failed: {}", stderr(&out));
    let imported: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("import --json stdout must be JSON");
    assert_eq!(imported["offline_profile_name"], "Imported");
    let edmx_file = imported["edmx_file"].as_str().unwrap();
    assert!(
        sb.path().join("offline").join(edmx_file).is_file(),
        "EDMX must land under SAP_ODATA_CONFIG_DIR"
    );
    assert!(sb.path().join("connections.toml").is_file());

    let out = sb.run(&["--json", "offline", "list"]);
    assert!(out.status.success(), "list failed: {}", stderr(&out));
    let buckets: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("list --json stdout must be JSON");
    assert_eq!(buckets.as_array().unwrap().len(), 1);
    assert_eq!(buckets[0]["name"], "Imported");
    assert_eq!(buckets[0]["service_count"], 1);
}
