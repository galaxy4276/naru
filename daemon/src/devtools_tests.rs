use uuid::Uuid;

use super::*;

#[test]
fn a_pr_takes_its_worst_check_and_the_panes_on_its_branch() {
    let pane = Uuid::new_v4();
    let json = r#"[
      {"number":313,"title":"a","url":"u1","headRefName":"feat/x","isDraft":false,"statusCheckRollup":[
        {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SKIPPED"},
        {"__typename":"StatusContext","state":"SUCCESS"},
        {"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE"}]},
      {"number":12,"title":"b","url":"u2","headRefName":"main","isDraft":true,"statusCheckRollup":[
        {"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""}]},
      {"number":9,"title":"c","url":"u3","headRefName":"y","isDraft":false,"statusCheckRollup":[
        {"__typename":"StatusContext","state":"SUCCESS"}]},
      {"number":8,"title":"d","url":"u4","headRefName":"z","isDraft":false,"statusCheckRollup":[]}
    ]"#;
    let prs = parse_prs(
        json,
        &[("feat/x".into(), pane), ("other".into(), Uuid::new_v4())],
    )
    .unwrap();
    let checks: Vec<_> = prs.iter().map(|pr| &pr.checks).collect();
    assert_eq!(
        checks,
        [
            &Checks::Fail,
            &Checks::Running,
            &Checks::Pass,
            &Checks::None
        ]
    );
    assert_eq!(prs[0].panes, [pane]);
    assert!(prs[1].draft && prs[1].panes.is_empty());
    assert!(parse_prs("not json", &[]).is_none());
}

#[test]
fn a_port_belongs_to_the_pane_whose_shell_started_its_program() {
    let ps = "  100     1   01:02:03 -zsh\n  400   100      05:00 pnpm start:dev\n  500   400   1-00:00:10 node dist/main.js --port 8044\n  700     1      00:09 vite\n  900   950      00:01 lone\n";
    let procs = parse_ps(ps);
    assert_eq!(procs[&500].age, 86_410);
    assert_eq!(procs[&500].command, "node dist/main.js --port 8044");
    let pane = Uuid::new_v4();
    let found = match_servers(
        &[(500, 8044), (700, 5173), (900, 7000)],
        &procs,
        &[(pane, 100)],
    );
    assert_eq!(
        found,
        [Server {
            port: 8044,
            pid: 500,
            command: "node dist/main.js --port 8044".into(),
            age: 86_410,
            pane
        }]
    );
}

#[test]
fn a_listening_socket_is_read_from_the_process_itself() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(listening_ports(std::process::id()).contains(&port));
}
