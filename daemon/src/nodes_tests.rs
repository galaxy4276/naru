use super::*;

#[test]
fn accepts_only_plain_hosts() {
    assert!(is_host("devbox"));
    assert!(is_host("me@host.example-1_x"));
    assert!(!is_host("-oProxyCommand=x"));
    assert!(!is_host("host;rm"));
    assert!(!is_host(""));
}

#[test]
fn keeps_only_connection_settings() {
    let config =
        "hostname 1.2.3.4\nport 22022\nlocalforward 5432 localhost:5432\nproxyjump none\nuser me\n";
    assert_eq!(
        ssh_options_from_config(config),
        ["hostname=1.2.3.4", "port=22022", "user=me"]
    );
}

#[test]
fn builds_the_ssh_line() {
    let line = ssh_command(
        "devbox",
        Some("~/projects/a b"),
        "claude --resume x",
        Path::new("/h/.naru/naru.sock"),
        &["port=22022".into()],
    )
    .unwrap();
    let words = shell_words::split(&line).unwrap();
    assert_eq!(
        words[..6],
        ["command", "ssh", "-F", "/dev/null", "-o", "port=22022"]
    );
    assert_eq!(words[words.len() - 2], "devbox");
    let remote = words.last().unwrap();
    assert!(remote.starts_with("cd ~/'projects/a b' && exec env NARU_SOCK=/tmp/naru-"));
    assert!(remote.contains("NARU_NODE=devbox"));
    let run = shell_words::split(remote).unwrap();
    assert_eq!(
        run.last().unwrap(),
        r#"claude --resume x; exec "$SHELL" -li"#
    );
    assert!(
        words
            .iter()
            .any(|w| w.ends_with(".sock:/h/.naru/naru.sock"))
    );
    assert!(ssh_command("-x", None, "", Path::new("/s"), &[]).is_err());
    let shell = ssh_command("devbox", None, "", Path::new("/s"), &[]).unwrap();
    let shell = shell_words::split(&shell).unwrap();
    assert!(shell.last().unwrap().ends_with(r#""$SHELL" -li"#));
}

#[test]
fn reads_the_prompt_signal_exactly() {
    assert!(is_prompt_signal(r#"{"op":"node_prompt"}"#));
    assert!(!is_prompt_signal(r#"{"op":"node_prompt","token":"x"}"#));
}
