use super::*;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_string()).collect()
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, "daily").unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn pmtui_arguments_parse_registry_socket_and_help() {
    let args = parse_args_from(&strings(&[
        "--registry",
        "/tmp/r.json",
        "--socket",
        "isolated",
    ]))
    .unwrap();
    assert_eq!(args.registry, PathBuf::from("/tmp/r.json"));
    assert_eq!(args.socket, "isolated");
    assert!(!args.help);

    assert!(parse_args_from(&strings(&["--help"])).unwrap().help);
}

#[test]
fn pmtui_arguments_accept_only_the_daily_channel() {
    let args = parse_args_from(&strings(&["--channel", "daily"])).unwrap();
    assert_eq!(args.channel, Some(Channel::Daily));

    let error = parse_args_from(&strings(&["--channel", "canary"])).unwrap_err();
    assert!(
        error.to_string().contains("--channel must be daily"),
        "{error:#}"
    );

    let error = parse_args_from(&strings(&["--daily"])).unwrap_err();
    assert!(error.to_string().contains("unknown argument"), "{error:#}");
}

#[test]
fn pmtui_arguments_reject_unknown_or_missing_values() {
    assert!(parse_args_from(&strings(&["--regsitry", "/tmp/r.json"])).is_err());
    assert!(parse_args_from(&strings(&["--registry"])).is_err());
    assert!(parse_args_from(&strings(&["--socket"])).is_err());
    assert!(parse_args_from(&strings(&["--channel"])).is_err());
}

#[test]
fn daily_channel_relaunches_the_local_debug_binary_without_the_channel_flag() {
    let dir = tempfile::tempdir().unwrap();
    let daily = dir.path().join("target/debug/pmtui");
    std::fs::create_dir_all(daily.parent().unwrap()).unwrap();
    make_executable(&daily);
    let current = dir.path().join("installed/pmtui");
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    make_executable(&current);

    let args = parse_args_from(&strings(&[
        "--registry",
        "/tmp/r.json",
        "--socket",
        "pm-test",
        "--channel",
        "daily",
    ]))
    .unwrap();

    assert_eq!(
        plan_daily_channel(&args, &current, dir.path()).unwrap(),
        DailyChannelPlan::Relaunch {
            exe: daily,
            argv: vec![
                "--registry".into(),
                "/tmp/r.json".into(),
                "--socket".into(),
                "pm-test".into(),
            ],
        }
    );
}

#[test]
fn daily_channel_runs_in_place_when_the_current_process_is_already_daily() {
    let dir = tempfile::tempdir().unwrap();
    let daily = dir.path().join("target/debug/pmtui");
    std::fs::create_dir_all(daily.parent().unwrap()).unwrap();
    make_executable(&daily);
    let args = parse_args_from(&strings(&["--channel", "daily"])).unwrap();

    assert_eq!(
        plan_daily_channel(&args, &daily, dir.path()).unwrap(),
        DailyChannelPlan::RunHere(Args {
            registry: default_registry_path(),
            socket: "pmd".into(),
            help: false,
            channel: None,
        })
    );
}

#[test]
fn daily_channel_requires_an_executable_local_build() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("installed/pmtui");
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    make_executable(&current);
    let args = parse_args_from(&strings(&["--channel", "daily"])).unwrap();

    let error = plan_daily_channel(&args, &current, dir.path()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("daily channel needs an executable local build"),
        "{error:#}"
    );

    let daily = dir.path().join("target/debug/pmtui");
    std::fs::create_dir_all(daily.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&daily).unwrap();

    let error = plan_daily_channel(&args, &current, dir.path()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("daily channel needs an executable local build"),
        "{error:#}"
    );

    std::fs::remove_dir_all(&daily).unwrap();
    std::fs::write(&daily, "not executable").unwrap();

    let error = plan_daily_channel(&args, &current, dir.path()).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("daily channel needs an executable local build"),
        "{error:#}"
    );
}

#[test]
fn daily_channel_stop_request_is_inert_when_the_daemon_is_already_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.json");
    let socket = "daily-stopped";

    request_daemon_stop_for_channel(&registry, socket, 1, Duration::ZERO).unwrap();

    assert!(
        !lease::daemon_stop_path(&registry, socket).exists(),
        "a free daemon lock proves there is no pmd to stop"
    );
}

#[test]
fn daily_channel_real_stop_request_uses_the_channel_stop_contract() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.json");
    let socket = "daily-real-stopped";

    request_daemon_stop_for_daily_channel(&registry, socket).unwrap();

    assert!(
        !lease::daemon_stop_path(&registry, socket).exists(),
        "the daily wrapper must keep the same inert free-lock behavior"
    );
}

#[test]
fn daily_channel_stop_request_waits_for_the_daemon_lock_to_release() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.json");
    let socket = "daily-release";
    let lock = lease::daemon_lock_path(&registry, socket);
    let held = lease::try_acquire(&lock)
        .unwrap()
        .expect("test owns daemon lock");
    let stop = lease::daemon_stop_path(&registry, socket);
    let stop_for_owner = stop.clone();
    let owner = std::thread::spawn(move || {
        for _ in 0..100 {
            if stop_for_owner.exists() {
                drop(held);
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("daily channel did not publish a daemon stop request");
    });

    request_daemon_stop_for_channel(&registry, socket, 100, Duration::from_millis(1)).unwrap();
    owner.join().unwrap();

    assert_eq!(
        std::fs::read_to_string(stop).unwrap(),
        format!("{}\n", std::process::id())
    );
}

#[test]
fn daily_channel_stop_request_reports_write_failure_and_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.json");

    let write_socket = "daily-write-failure";
    let write_lock = lease::daemon_lock_path(&registry, write_socket);
    let _write_hold = lease::try_acquire(&write_lock)
        .unwrap()
        .expect("test owns daemon lock");
    let write_stop = lease::daemon_stop_path(&registry, write_socket);
    std::fs::create_dir_all(&write_stop).unwrap();

    let error =
        request_daemon_stop_for_channel(&registry, write_socket, 1, Duration::ZERO).unwrap_err();
    assert!(error.to_string().contains("write"), "{error:#}");

    let timeout_socket = "daily-timeout";
    let timeout_lock = lease::daemon_lock_path(&registry, timeout_socket);
    let _timeout_hold = lease::try_acquire(&timeout_lock)
        .unwrap()
        .expect("test owns daemon lock");

    let error =
        request_daemon_stop_for_channel(&registry, timeout_socket, 1, Duration::ZERO).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pmd did not stop after daily channel request"),
        "{error:#}"
    );
}

#[test]
fn daily_channel_relaunch_plan_stops_then_execs_the_selected_binary() {
    let dir = tempfile::tempdir().unwrap();
    let args = Args {
        registry: dir.path().join("registry.json"),
        socket: "daily-relaunch".into(),
        help: false,
        channel: Some(Channel::Daily),
    };
    let exe = dir.path().join("target/debug/pmtui");
    let argv = vec!["--registry".into(), args.registry.as_os_str().into()];
    let stopped = std::cell::RefCell::new(Vec::<(PathBuf, String)>::new());
    let executed = std::cell::RefCell::new(Vec::<(PathBuf, Vec<OsString>)>::new());
    let mut dashboard = |_app: &mut App| panic!("relaunch must not start the current dashboard");
    let mut confirm = |_prompt: &str| panic!("relaunch must not ask takeover questions");
    let mut force = |_registry: &Path, _socket: &str, _owner: &DashboardOwner| {
        panic!("relaunch must not force a dashboard takeover")
    };

    let error = execute_daily_channel_plan(
        args.clone(),
        DailyChannelPlan::Relaunch {
            exe: exe.clone(),
            argv: argv.clone(),
        },
        &mut dashboard,
        &mut confirm,
        &mut force,
        &mut |registry, socket| {
            stopped
                .borrow_mut()
                .push((registry.to_path_buf(), socket.to_string()));
            Ok(())
        },
        &mut |exe, argv| {
            executed
                .borrow_mut()
                .push((exe.to_path_buf(), argv.to_vec()));
            anyhow::bail!("exec refused")
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("exec refused"), "{error:#}");
    assert_eq!(
        stopped.into_inner(),
        vec![(args.registry.clone(), args.socket.clone())]
    );
    assert_eq!(executed.into_inner(), vec![(exe, argv)]);
}

#[test]
fn daily_channel_relaunch_plan_does_not_exec_when_stop_fails() {
    let dir = tempfile::tempdir().unwrap();
    let args = Args {
        registry: dir.path().join("registry.json"),
        socket: "daily-relaunch".into(),
        help: false,
        channel: Some(Channel::Daily),
    };
    let executed = std::cell::Cell::new(false);
    let mut dashboard = |_app: &mut App| panic!("relaunch must not start the current dashboard");
    let mut confirm = |_prompt: &str| panic!("relaunch must not ask takeover questions");
    let mut force = |_registry: &Path, _socket: &str, _owner: &DashboardOwner| {
        panic!("relaunch must not force a dashboard takeover")
    };

    let error = execute_daily_channel_plan(
        args,
        DailyChannelPlan::Relaunch {
            exe: dir.path().join("target/debug/pmtui"),
            argv: Vec::new(),
        },
        &mut dashboard,
        &mut confirm,
        &mut force,
        &mut |_registry, _socket| anyhow::bail!("stop refused"),
        &mut |_exe, _argv| {
            executed.set(true);
            Ok(())
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("stop refused"), "{error:#}");
    assert!(!executed.get());
}

#[test]
fn daily_channel_run_here_uses_the_normal_dashboard_startup() {
    let dir = tempfile::tempdir().unwrap();
    let args = Args {
        registry: dir.path().join("registry.json"),
        socket: "daily-here".into(),
        help: false,
        channel: None,
    };
    let expected_registry = args.registry.clone();
    let expected_socket = args.socket.clone();
    let dashboard_called = std::cell::Cell::new(false);
    let mut dashboard = |app: &mut App| {
        assert_eq!(app.registry_path, expected_registry);
        assert_eq!(app.socket, expected_socket);
        dashboard_called.set(true);
        app.should_quit = true;
        Ok(())
    };
    let mut confirm = |_prompt: &str| panic!("a free dashboard lock must not prompt");
    let mut force = |_registry: &Path, _socket: &str, _owner: &DashboardOwner| {
        panic!("a free dashboard lock must not force takeover")
    };

    execute_daily_channel_plan(
        args.clone(),
        DailyChannelPlan::RunHere(args),
        &mut dashboard,
        &mut confirm,
        &mut force,
        &mut |_registry, _socket| panic!("run-here path must not stop pmd"),
        &mut |_exe, _argv| panic!("run-here path must not exec"),
    )
    .unwrap();

    assert!(dashboard_called.get());
}

#[test]
fn daily_channel_start_uses_the_injected_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let daily = dir.path().join("target/debug/pmtui");
    std::fs::create_dir_all(daily.parent().unwrap()).unwrap();
    make_executable(&daily);
    let current = dir.path().join("installed/pmtui");
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    make_executable(&current);
    let args = Args {
        registry: dir.path().join("registry.json"),
        socket: "daily-start".into(),
        help: false,
        channel: Some(Channel::Daily),
    };
    let stopped = std::cell::RefCell::new(Vec::<(PathBuf, String)>::new());
    let executed = std::cell::RefCell::new(Vec::<(PathBuf, Vec<OsString>)>::new());
    let mut dashboard = |_app: &mut App| panic!("relaunch must not start the current dashboard");
    let mut confirm = |_prompt: &str| panic!("relaunch must not ask takeover questions");
    let mut force = |_registry: &Path, _socket: &str, _owner: &DashboardOwner| {
        panic!("relaunch must not force a dashboard takeover")
    };

    start_daily_channel(
        args.clone(),
        &mut dashboard,
        &mut confirm,
        &mut force,
        &DailyChannelRuntime {
            current_exe: current,
            cwd: dir.path().to_path_buf(),
        },
        &mut |registry, socket| {
            stopped
                .borrow_mut()
                .push((registry.to_path_buf(), socket.to_string()));
            Ok(())
        },
        &mut |exe, argv| {
            executed
                .borrow_mut()
                .push((exe.to_path_buf(), argv.to_vec()));
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(stopped.into_inner(), vec![(args.registry, args.socket)]);
    assert_eq!(
        executed.into_inner(),
        vec![(
            daily,
            vec![
                "--registry".into(),
                dir.path().join("registry.json").as_os_str().into(),
                "--socket".into(),
                "daily-start".into(),
            ],
        )]
    );
}

#[test]
fn daily_channel_startup_uses_the_real_startup_branch_when_the_selected_build_matches() {
    let dir = tempfile::tempdir_in("target").unwrap();
    let daily = dir.path().join("target/debug/pmtui");
    std::fs::create_dir_all(daily.parent().unwrap()).unwrap();
    let current = std::env::current_exe().unwrap();
    std::fs::hard_link(&current, &daily).unwrap();
    let _runtime = set_daily_channel_runtime_for_test(current, dir.path().to_path_buf());
    let registry = dir.path().join("registry.json");
    let called = std::cell::Cell::new(false);
    let mut confirm = |_prompt: &str| panic!("a free dashboard lock must not prompt");
    let mut force = |_registry: &Path, _socket: &str, _owner: &DashboardOwner| {
        panic!("a free dashboard lock must not force takeover")
    };

    start_app_with_takeover(
        Args {
            registry: registry.clone(),
            socket: "daily-real-startup".into(),
            help: false,
            channel: Some(Channel::Daily),
        },
        &mut |app| {
            called.set(true);
            assert_eq!(app.registry_path, registry);
            assert_eq!(app.socket, "daily-real-startup");
            Ok(())
        },
        &mut confirm,
        &mut force,
    )
    .unwrap();

    assert!(called.get());
}

#[test]
fn exec_dashboard_result_reports_the_exec_target() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("missing-pmtui");

    let error = exec_dashboard_result(&exe, &[]).unwrap_err();

    assert!(
        format!("{error:#}").contains(&format!("exec {}", exe.display())),
        "{error:#}"
    );
}

#[test]
fn session_start_hook_records_identity_without_claiming_a_completed_turn() {
    // Fork publication needs the assigned child id before any agent turn has run.
    let args = strings(&["hook", "session-start"]);
    assert_eq!(parse_action(&args).unwrap(), Action::HookSessionStart);
    let dir = tempfile::tempdir().unwrap();
    let paths = ProjectPaths::for_session(dir.path(), "bot");
    std::fs::create_dir_all(paths.state_dir()).unwrap();
    let env = |key: &str| {
        (key == tmux::ENV_STATE_DIR).then(|| paths.state_dir().to_string_lossy().into_owned())
    };
    record_session_identity(&env, TRAE_STOP_PAYLOAD).unwrap();
    assert_eq!(
        read_captured_conversation_id(&paths).as_deref(),
        Some("01a133c3-b12f-7033-ae77-38f3294fb68e")
    );
    assert!(!paths.turn_signal().exists());
    record_session_identity(&env, r#"{"session_id":"invalid"}"#).unwrap();
    assert_eq!(
        read_captured_conversation_id(&paths).as_deref(),
        Some("01a133c3-b12f-7033-ae77-38f3294fb68e")
    );
}

#[test]
fn hook_turn_complete_appends_to_the_managed_state_dir() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join(".project-state/sessions/bot");
    std::fs::create_dir_all(&state_dir).unwrap();
    let env =
        |key: &str| (key == tmux::ENV_STATE_DIR).then(|| state_dir.to_string_lossy().into_owned());

    run_turn_complete_hook(&env, "").unwrap();
    run_turn_complete_hook(&env, "").unwrap();

    assert_eq!(
        std::fs::read(state_dir.join(".daemon/turn-complete")).unwrap(),
        b".."
    );
}

#[test]
fn hook_turn_complete_requires_a_managed_state_dir() {
    let error = run_turn_complete_hook(&|_| None, "").unwrap_err();
    assert!(error.to_string().contains(tmux::ENV_STATE_DIR), "{error:#}");
}

/// A real TraeX `Stop` payload (traex 0.209.1), trimmed: `session_id` is the THREAD id, and
/// `thread_name` is the `--session-id` pmtui gave it, which `traex resume` cannot open.
const TRAE_STOP_PAYLOAD: &str = r#"{"session_id":"01a133c3-b12f-7033-ae77-38f3294fb68e","thread_name":"40d6555b-b1aa-45f8-9d23-20594c062543","session_name":"40d6555b-b1aa-45f8-9d23-20594c062543","hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"ok"}"#;

#[test]
fn hook_turn_complete_records_the_thread_id_trae_reported() {
    let dir = tempfile::tempdir().unwrap();
    let paths = agent_manager::state::ProjectPaths::for_session(dir.path(), "bot");
    let state_dir = paths.state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let env =
        |key: &str| (key == tmux::ENV_STATE_DIR).then(|| state_dir.to_string_lossy().into_owned());

    run_turn_complete_hook(&env, TRAE_STOP_PAYLOAD).unwrap();

    assert_eq!(
        agent_manager::state::codex_identity::read(&paths).as_deref(),
        Some("01a133c3-b12f-7033-ae77-38f3294fb68e"),
        "the id lands where pmd and pmtui read it"
    );
    assert_eq!(std::fs::read(paths.turn_signal()).unwrap(), b".");
}

#[test]
fn hook_turn_complete_signals_the_turn_without_a_usable_id() {
    let dir = tempfile::tempdir().unwrap();
    let paths = agent_manager::state::ProjectPaths::for_session(dir.path(), "bot");
    let state_dir = paths.state_dir();
    std::fs::create_dir_all(&state_dir).unwrap();
    let env =
        |key: &str| (key == tmux::ENV_STATE_DIR).then(|| state_dir.to_string_lossy().into_owned());

    for payload in [
        "",
        "not json",
        r#"{"session_id":"not-a-uuid"}"#,
        r#"{"thread_name":"x"}"#,
    ] {
        run_turn_complete_hook(&env, payload).unwrap();
    }

    assert_eq!(agent_manager::state::codex_identity::read(&paths), None);
    assert_eq!(std::fs::read(paths.turn_signal()).unwrap(), b"....");
}
