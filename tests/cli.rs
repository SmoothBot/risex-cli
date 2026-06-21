use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_flag_prints_semver() {
    Command::cargo_bin("risex")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("risex"));
}

#[test]
fn help_lists_binary_name() {
    Command::cargo_bin("risex")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("risex"));
}

#[test]
fn completions_emit_a_script_with_subcommands() {
    Command::cargo_bin("risex")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_risex").and(predicate::str::contains("orderbook")));
}
