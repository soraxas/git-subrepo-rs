use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-subrepo"))
        .args(args)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap()
}

#[test]
fn legacy_parse_errors_keep_their_messages_and_exit_status() {
    for (args, expected) in [
        (
            vec!["main", "1"],
            "'main' is not a command. See 'git subrepo help'.",
        ),
        (vec!["clone", "--foo"], "error: unknown option `foo'"),
        (vec!["pull", "--foo"], "error: unknown option `foo'"),
        (vec!["pull", "--foo=value"], "error: unknown option `foo'"),
        (vec!["commit"], "Command 'commit' requires arg 'subdir'."),
        (
            vec!["config", "lib"],
            "Command 'config' requires arg 'key'.",
        ),
        (
            vec!["clone", "--all"],
            "Invalid option '--all' for 'clone'.",
        ),
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("git-subrepo: {expected}\n"),
            "{args:?}"
        );
    }
}

#[test]
fn version_remains_a_plain_number_without_a_repository() {
    for args in [vec!["--version"], vec!["pull", "--version"]] {
        let output = run(&args);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            concat!(env!("CARGO_PKG_VERSION"), "\n")
        );
    }
}

#[test]
fn help_is_successful_and_written_to_stdout_without_a_repository() {
    for args in [vec!["--help"], vec!["pull", "--help"], vec!["help", "pull"]] {
        let output = run(&args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Usage:"),
            "{args:?}"
        );
    }
}

#[test]
fn completion_scripts_are_available_without_a_repository() {
    for shell in ["fish", "bash", "zsh"] {
        let output = run(&["completion", shell]);
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.is_empty());
    }
}

#[test]
fn boolean_flags_reject_attached_values_before_dispatch() {
    for (command, flag) in [
        ("pull", "force"),
        ("pull", "quiet"),
        ("pull", "update"),
        ("pull", "stage-only"),
        ("push", "squash"),
        ("workon", "no-shell"),
        ("status", "no-fetch"),
        ("status", "no-dirty"),
        ("pull", "version"),
    ] {
        for value in ["false", "true", "bogus", ""] {
            let option = format!("--{flag}={value}");
            let output = run(&[command, &option, "--version"]);
            assert_eq!(output.status.code(), Some(1), "{command} {option}");
            assert!(output.stdout.is_empty(), "{command} {option}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected value"));
        }
    }
}

#[test]
fn clone_keeps_legacy_trailing_argument_and_hyphen_value_handling() {
    for extras in [
        vec!["--foo"],
        vec!["-Z", "--message", "x"],
        vec!["extra", "--quiet=false"],
    ] {
        let mut args = vec!["clone", "remote", "lib"];
        args.extend(&extras);
        let output = run(&args);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "git-subrepo: Unknown argument(s) '{}' for 'clone' command.\n",
                extras.join(" ")
            )
        );
    }
    for option in ["--message", "--branch", "--method", "-m", "-qm"] {
        let output = run(&["--version", "clone", "remote", "lib", option, "-hello"]);
        assert!(
            output.status.success(),
            "{option}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for args in [
        vec!["--version", "clone", "--message", "-hello", "remote", "lib"],
        vec![
            "--version",
            "clone",
            "remote",
            "lib",
            "--message",
            "--stage-only",
        ],
    ] {
        assert_eq!(run(&args).status.code(), Some(1), "{args:?}");
    }
    assert!(
        run(&["pull", "--message=--force=false", "--version"])
            .status
            .success()
    );
    assert!(
        run(&["pull", "--version", "--", "--force=false"])
            .status
            .success()
    );
}

struct TestRepo(std::path::PathBuf);

impl TestRepo {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "git-subrepo-completion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "-q"]);
        repo
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn completion_output(dir: &std::path::Path, line: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_git-subrepo"))
        .args(["__complete_word__", "--shell", "fish", "--line", line])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn candidates(dir: &std::path::Path, line: &str) -> Vec<String> {
    completion_output(dir, line)
        .lines()
        .filter(|line| !line.starts_with('\u{1}'))
        .map(|line| line.split('\t').next().unwrap().to_string())
        .collect()
}

#[test]
fn workon_completion_never_falls_back_to_files() {
    let repo = TestRepo::new();
    std::fs::write(repo.0.join("notes.txt"), "unrelated file").unwrap();
    for name in ["lib/alpha", "lib/alpha/nested", "space dir", "untracked"] {
        let dir = repo.0.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitrepo"), "[subrepo]\n").unwrap();
    }
    assert_eq!(completion_output(&repo.0, "git-subrepo workon "), "");
    repo.git(&["add", "lib", "space dir"]);
    assert_eq!(
        candidates(&repo.0, "git-subrepo workon "),
        ["lib/alpha", "lib/alpha/nested", "space dir"]
    );
    for line in [
        "git-subrepo workon notes",
        "git-subrepo workon untracked",
        "git-subrepo -q workon --no-shell notes",
        "git-subrepo workon -u https://example.com/fork notes",
    ] {
        assert_eq!(completion_output(&repo.0, line), "", "{line}");
    }
    // Commands accepting arbitrary directories still need filesystem completion.
    for line in ["git-subrepo init ", "git-subrepo clone remote "] {
        assert!(completion_output(&repo.0, line).contains('\u{1}'), "{line}");
    }
}

#[test]
fn completion_reads_tracked_nested_subrepos_without_mutating_the_index() {
    let repo = TestRepo::new();
    for name in ["lib/alpha", "lib/alpha/nested", "space dir", "untracked"] {
        let dir = repo.0.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitrepo"), "[subrepo]\nmethod = merge\n").unwrap();
    }
    repo.git(&["add", "lib", "space dir"]);
    let before = std::fs::read(repo.0.join(".git/index")).unwrap();
    for line in [
        "git-subrepo pull ",
        "git-subrepo workon -u https://example.com/fork ",
        "git-subrepo -q push --branch=main ",
    ] {
        let values = candidates(&repo.0, line);
        for expected in ["lib/alpha", "lib/alpha/nested", "space dir"] {
            assert!(
                values.iter().any(|value| value == expected),
                "{line}: {values:?}"
            );
        }
        assert!(!values.iter().any(|value| value == "untracked"));
    }
    assert_eq!(before, std::fs::read(repo.0.join(".git/index")).unwrap());
}

#[test]
fn completion_understands_option_values_and_config_positions_without_a_repository() {
    let dir = std::env::temp_dir();
    for (line, expected, absent) in [
        ("git-subrepo pull --method ", "rebase", "pull"),
        ("git-subrepo pull --method=", "--method=merge", "pull"),
        ("git-subrepo config 'space dir' ", "parent", "rebase"),
        ("git-subrepo config lib method ", "merge", "remote"),
        ("git-subrepo config -f lib method ", "merge", "remote"),
        ("git-subrepo config lib -qf method ", "merge", "remote"),
        ("git-subrepo workon --", "--upstream", "--update"),
        ("git-subrepo push --", "--update", "--upstream"),
    ] {
        let values = candidates(&dir, line);
        assert!(
            values.iter().any(|value| value == expected),
            "{line}: {values:?}"
        );
        assert!(
            !values.iter().any(|value| value == absent),
            "{line}: {values:?}"
        );
    }
    assert!(candidates(&dir, "git-subrepo pull lib/").is_empty());
}

#[test]
fn named_config_completion_tolerates_an_empty_command_line() {
    let output = run(&[
        "__complete_word__",
        "--shell",
        "fish",
        "--candidates",
        "VALUE",
        "--line",
        "",
    ]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn commit_completion_offers_local_refs() {
    let repo = TestRepo::new();
    repo.git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "--no-gpg-sign",
        "--allow-empty",
        "-m",
        "initial",
    ]);
    repo.git(&["branch", "topic"]);
    repo.git(&["update-ref", "refs/subrepo/lib/branch", "HEAD"]);
    let values = candidates(&repo.0, "git-subrepo commit lib ");
    assert!(values.iter().any(|value| value == "topic"), "{values:?}");
    assert!(
        values.iter().any(|value| value == "subrepo/lib/branch"),
        "{values:?}"
    );
}

fn fish_candidates(dir: &std::path::Path, line: &str) -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let binary_dir = std::path::Path::new(env!("CARGO_BIN_EXE_git-subrepo"))
        .parent()
        .unwrap();
    let output = Command::new("fish")
        .args([
            "--no-config",
            "-c",
            "set -p PATH $argv[1]; set -p fish_complete_path $argv[2]; complete -C \"$argv[3]\"",
            "--",
        ])
        .arg(binary_dir)
        .arg(root.join("etc"))
        .arg(line)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|line| line.split('\t').next().unwrap().to_owned())
        .collect()
}

#[test]
fn bundled_fish_script_completes_both_invocation_forms() {
    if Command::new("fish").arg("--version").output().is_err() {
        eprintln!("skipping Fish integration check: fish is not installed");
        return;
    }
    let repo = TestRepo::new();
    std::fs::create_dir(repo.0.join("space dir")).unwrap();
    std::fs::write(repo.0.join("space dir/.gitrepo"), "[subrepo]\n").unwrap();
    repo.git(&["add", "."]);
    for prefix in ["git-subrepo", "git subrepo"] {
        for (suffix, expected) in [
            ("wor", "workon"),
            ("pull ", "space dir"),
            ("pull --method=", "--method=merge"),
            ("config -f 'space dir' method ", "rebase"),
        ] {
            let line = format!("{prefix} {suffix}");
            let values = fish_candidates(&repo.0, &line);
            assert!(
                values.iter().any(|value| value == expected),
                "{line}: {values:?}"
            );
        }
    }
}

#[test]
fn fish_workon_offers_only_matching_subrepos() {
    if Command::new("fish").arg("--version").output().is_err() {
        eprintln!("skipping Fish integration check: fish is not installed");
        return;
    }
    let repo = TestRepo::new();
    std::fs::write(repo.0.join("notes.txt"), "unrelated file").unwrap();
    for name in ["lib/alpha", "lib/alpha/nested", "space dir", "untracked"] {
        let dir = repo.0.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitrepo"), "[subrepo]\n").unwrap();
    }
    for prefix in ["git-subrepo", "git subrepo"] {
        assert!(fish_candidates(&repo.0, &format!("{prefix} workon ")).is_empty());
    }
    repo.git(&["add", "lib", "space dir"]);
    for prefix in ["git-subrepo", "git subrepo"] {
        for (suffix, expected) in [
            ("", vec!["lib/alpha", "lib/alpha/nested", "space dir"]),
            ("lib/", vec!["lib/alpha", "lib/alpha/nested"]),
            ("notes", vec![]),
            ("untracked", vec![]),
            ("--no", vec!["--no-edit", "--no-shell"]),
        ] {
            let line = format!("{prefix} workon {suffix}");
            assert_eq!(fish_candidates(&repo.0, &line), expected, "{line}");
        }
    }
}
