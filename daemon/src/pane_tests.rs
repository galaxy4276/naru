use std::sync::mpsc::channel;
use std::time::Duration;

use super::*;

#[test]
fn reports_osc_and_exit_and_replays_output() {
    let (events, received) = channel();
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        r#"printf 'hello\033]777;notify;warp://cli-agent;{"a":1}\007'"#,
    ]);
    let pane = Pane::spawn(command, PtySize::default(), move |event| {
        let _ = events.send(match event {
            PaneEvent::Osc(osc) => format!("{osc:?}"),
            PaneEvent::Exited => "exit".to_owned(),
        });
    })
    .unwrap();
    let wait = || received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        wait(),
        r#"Notify { title: "warp://cli-agent", body: "{\"a\":1}" }"#
    );
    assert_eq!(wait(), "exit");
    let (replay, _) = pane.attach();
    assert!(String::from_utf8_lossy(&replay).starts_with("hello"));
}

#[test]
fn keeps_agent_session_env_out_of_panes() {
    for key in [
        "CLAUDECODE",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_MESSAGING_TOKEN",
        "CLAUDE_PID",
    ] {
        assert!(is_agent_session_env(key), "{key}");
    }
    for key in ["CLAUDE_CODEX_REVIEW", "PATH", "ANTHROPIC_API_KEY"] {
        assert!(!is_agent_session_env(key), "{key}");
    }
    let command = shell_command(Path::new("/tmp"), Path::new("/tmp/zsh"));
    assert_eq!(command.get_env("TERM_PROGRAM").unwrap(), "WarpTerminal");
    assert_eq!(command.get_env("NARU_PANE").unwrap(), "1");
}
