use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ii"))
}

#[test]
fn help_is_noninteractive_and_stderr_is_empty() {
    let result = binary().arg("--help").output().unwrap();
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    assert!(String::from_utf8_lossy(&result.stdout).contains("SHELL SETUP"));
}

#[test]
fn version_is_machine_readable() {
    let result = binary().arg("--version").output().unwrap();
    assert!(result.status.success());
    assert_eq!(
        result.stdout,
        format!("ii {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
}

#[test]
fn generates_all_shell_integrations_without_a_tty() {
    for (shell, expected) in [
        ("bash", "builtin cd"),
        ("zsh", "builtin cd"),
        ("fish", "string split0"),
        ("powershell", "LiteralPath"),
    ] {
        let result = binary().args(["init", shell]).output().unwrap();
        assert!(result.status.success(), "{shell}");
        assert!(String::from_utf8_lossy(&result.stdout).contains(expected));
        assert!(result.stderr.is_empty());
    }
}

#[test]
fn refuses_noninteractive_input_without_printing_a_fake_path() {
    let result = binary().output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(!result.stderr.contains(&0x1b));
}

#[test]
fn bad_arguments_have_a_distinct_exit_code() {
    for arguments in [
        vec!["--typo"],
        vec!["init"],
        vec!["init", "unknown"],
        vec!["one", "two"],
    ] {
        let result = binary().args(arguments).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
    }
}
