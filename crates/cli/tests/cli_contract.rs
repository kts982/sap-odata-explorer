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

/// A local URL nothing listens on, so every request fails fast with
/// "connection refused" — exercises error paths without a SAP system.
fn closed_local_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

// ── stdout / stderr separation ──

#[test]
fn verbose_logs_never_reach_stdout() {
    let sb = Sandbox::new("verbose_stdout");
    let out = sb.run(&["-v", "--json", "offline", "list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout(&out));
    assert!(
        parsed.is_ok(),
        "stdout must be pure JSON with -v, got:\n{}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("DEBUG"),
        "debug logs belong on stderr"
    );
}

#[test]
fn services_fails_when_every_catalog_fails() {
    // Both catalogs unreachable must not look like "no services exist".
    let sb = Sandbox::new("services_fail");
    let url = closed_local_url();
    let out = sb.run(&[
        "--url",
        &url,
        "--user",
        "u",
        "--password",
        "p",
        "--json",
        "services",
    ]);
    assert!(!out.status.success(), "must exit non-zero");
    assert!(
        stdout(&out).trim().is_empty(),
        "no data on stdout, got:\n{}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("could not fetch the service catalog"),
        "{}",
        stderr(&out)
    );
}

// ── --json on local (no-network) commands ──

/// Browser SSO profile: no keyring lookup happens for it, so listing it
/// never touches the developer's OS credential store.
const BROWSER_SSO_CONFIG: &str = r#"
[connections.DEV]
base_url = "https://dev.example.com:44301"
client = "100"
language = "DE"
browser_sso = true

[connections.DEV.aliases]
wo = "/sap/opu/odata/sap/ZWAREHOUSE_ORDER_SRV"
"#;

#[test]
fn profile_list_json_on_empty_config_is_an_empty_array() {
    let sb = Sandbox::new("profile_list_empty");
    let out = sb.run(&["--json", "profile", "list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    assert_eq!(v, serde_json::json!([]));
}

#[test]
fn profile_list_json_shape() {
    let sb = Sandbox::new("profile_list_json");
    sb.write_file("connections.toml", BROWSER_SSO_CONFIG);
    let out = sb.run(&["--json", "profile", "list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    let p = &v[0];
    assert_eq!(p["name"], "DEV");
    assert_eq!(p["base_url"], "https://dev.example.com:44301");
    assert_eq!(p["client"], "100");
    assert_eq!(p["language"], "DE");
    assert_eq!(p["auth"], "browser_sso");
    assert_eq!(p["password_source"], "browser_sso");
    assert_eq!(
        p["aliases"]["wo"],
        "/sap/opu/odata/sap/ZWAREHOUSE_ORDER_SRV"
    );
    assert!(p.get("password").is_none(), "never emit a password field");
}

#[test]
fn profile_where_json_reports_the_active_dir() {
    let sb = Sandbox::new("profile_where_json");
    sb.write_file("connections.toml", BROWSER_SSO_CONFIG);
    let out = sb.run(&["--json", "profile", "where"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    assert_eq!(v["portable"], false);
    assert_eq!(v["config_file_exists"], true);
    let reported = std::path::PathBuf::from(v["path"].as_str().unwrap());
    assert_eq!(reported, sb.path());
}

#[test]
fn alias_list_json_shape() {
    let sb = Sandbox::new("alias_list_json");
    sb.write_file("connections.toml", BROWSER_SSO_CONFIG);
    let out = sb.run(&["-p", "DEV", "--json", "alias", "list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    assert_eq!(
        v,
        serde_json::json!([{ "name": "wo", "path": "/sap/opu/odata/sap/ZWAREHOUSE_ORDER_SRV" }])
    );
}

#[test]
fn missing_profile_fails_with_stderr_only() {
    let sb = Sandbox::new("missing_profile");
    let out = sb.run(&[
        "-p",
        "NOPE",
        "--json",
        "-s",
        "/sap/opu/odata/sap/X",
        "entities",
    ]);
    assert!(!out.status.success());
    assert!(stdout(&out).trim().is_empty(), "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("NOPE"), "{}", stderr(&out));
}

#[test]
fn global_profile_flag_on_offline_list_explains_the_clash() {
    let sb = Sandbox::new("offline_p_clash");
    sb.write_file("connections.toml", BROWSER_SSO_CONFIG);
    let edmx = sb.write_file("contract.edmx", MINIMAL_V4_EDMX);
    assert!(
        sb.run(&["offline", "import", edmx.to_str().unwrap()])
            .status
            .success()
    );

    let out = sb.run(&["-p", "DEV", "offline", "list"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("is a connected profile, not an offline bucket"),
        "{err}"
    );
    assert!(
        err.contains("Imported"),
        "should list available buckets: {err}"
    );
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

// ── build ──

/// `build` with a full service path sends no request at all, so a
/// closed local URL is enough.
fn build_url(sb: &Sandbox, service: &str, extra: &[&str]) -> String {
    let url = closed_local_url();
    let mut args = vec![
        "--url",
        &url,
        "--user",
        "u",
        "--password",
        "p",
        "-s",
        service,
        "build",
        "Items",
    ];
    args.extend_from_slice(extra);
    let out = sb.run(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    stdout(&out).trim().to_string()
}

#[test]
fn build_count_uses_the_version_of_the_service_path() {
    let sb = Sandbox::new("build_count");
    let v4 = build_url(
        &sb,
        "/sap/opu/odata4/sap/zsrv/srvd/sap/zsrv/0001",
        &["--count"],
    );
    assert!(v4.contains("$count=true"), "{v4}");
    assert!(!v4.contains("inlinecount"), "{v4}");
    let v2 = build_url(&sb, "/sap/opu/odata/sap/ZSRV", &["--count"]);
    assert!(v2.contains("$inlinecount=allpages"), "{v2}");
}

#[test]
fn build_json_is_a_json_string() {
    let sb = Sandbox::new("build_json");
    let url = closed_local_url();
    let out = sb.run(&[
        "--url",
        &url,
        "--user",
        "u",
        "--password",
        "p",
        "--json",
        "-s",
        "/sap/opu/odata/sap/ZSRV",
        "build",
        "Items",
        "--top",
        "5",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON");
    assert!(
        v.as_str()
            .unwrap()
            .starts_with("/sap/opu/odata/sap/ZSRV/Items?"),
        "{v}"
    );
}

// ── profile add re-runs (no keyring writes) ──
//
// Profile names are deliberately unusual: session / keyring cleanup is
// keyed by profile name in the real OS keyring, which a config-dir
// sandbox doesn't isolate. None of these cases writes a credential.

fn profile_json(sb: &Sandbox, name: &str) -> serde_json::Value {
    let out = sb.run(&["--json", "profile", "list"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let all: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    all.as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .cloned()
        .expect("profile listed")
}

#[test]
fn profile_add_rerun_keeps_browser_sso_mode() {
    let sb = Sandbox::new("rerun_browser");
    sb.write_file(
        "connections.toml",
        "[connections.ZZ_CONTRACT_BROWSER_7F3A]\nbase_url = \"https://old.example.com\"\nbrowser_sso = true\n",
    );
    let out = sb.run(&[
        "profile",
        "add",
        "ZZ_CONTRACT_BROWSER_7F3A",
        "--url",
        "https://new.example.com",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let p = profile_json(&sb, "ZZ_CONTRACT_BROWSER_7F3A");
    assert_eq!(p["auth"], "browser_sso", "{p}");
    assert_eq!(p["base_url"], "https://new.example.com");
}

#[test]
fn profile_add_blank_password_keeps_plaintext_password_and_insecure_tls() {
    let sb = Sandbox::new("rerun_plaintext");
    sb.write_file(
        "connections.toml",
        "[connections.ZZ_CONTRACT_PLAIN_7F3A]\nbase_url = \"https://dev.example.com\"\nusername = \"u\"\npassword = \"secret\"\ninsecure_tls = true\n",
    );
    let out = sb.run(&[
        "profile",
        "add",
        "ZZ_CONTRACT_PLAIN_7F3A",
        "--url",
        "https://dev.example.com",
        "--user",
        "u",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("Password unchanged"),
        "{}",
        stdout(&out)
    );
    let p = profile_json(&sb, "ZZ_CONTRACT_PLAIN_7F3A");
    assert_eq!(p["password_source"], "plaintext", "{p}");
    assert_eq!(p["insecure_tls"], true, "{p}");
}

#[test]
fn profile_add_without_password_does_not_claim_a_keyring_write() {
    let sb = Sandbox::new("add_no_password");
    let out = sb.run(&[
        "profile",
        "add",
        "ZZ_CONTRACT_NOPW_7F3A",
        "--url",
        "https://dev.example.com",
        "--user",
        "u",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(!text.contains("stored in OS keyring"), "{text}");
    assert!(text.contains("No password stored"), "{text}");
}
