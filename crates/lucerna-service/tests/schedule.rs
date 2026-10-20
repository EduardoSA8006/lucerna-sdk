use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use lucerna_paths::Env;
use lucerna_service::testing::FakeScheduler;
use lucerna_service::{
    ScheduleError, Scheduler, SystemdScheduler, TimerSpec, calendar_utc, render_timer,
};

fn spec(name: &str, at_unix: u64, persistent: bool) -> TimerSpec {
    TimerSpec {
        name: name.into(),
        at_unix,
        persistent,
    }
}

#[test]
fn calendar_values() {
    assert_eq!(calendar_utc(0), "1970-01-01 00:00:00 UTC");
    assert_eq!(calendar_utc(951_782_400), "2000-02-29 00:00:00 UTC");
    assert_eq!(calendar_utc(1_700_000_000), "2023-11-14 22:13:20 UTC");
}

#[test]
fn renders_persistent_and_runtime_timers() {
    let p = render_timer("routine", &spec("lembrete", 1_700_000_000, true));
    assert_eq!(
        p,
        "[Unit]\nDescription=Lucerna: routine (lembrete)\n\n[Timer]\n\
         OnCalendar=2023-11-14 22:13:20 UTC\nAccuracySec=1s\n\
         Unit=lucerna-routine.service\nPersistent=true\n\n\
         [Install]\nWantedBy=timers.target\n"
    );
    let r = render_timer("routine", &spec("x", 0, false));
    assert!(r.contains("OnCalendar=1970-01-01 00:00:00 UTC\n"));
    assert!(r.contains("Unit=lucerna-routine.service\n"));
    assert!(!r.contains("Persistent") && !r.contains("[Install]"));
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Um `systemctl` falso que registra os argumentos em `dir/log` e depois roda
/// `body`.
fn script(dir: &Path, body: &str) -> (PathBuf, PathBuf) {
    let log = dir.join("log");
    let bin = dir.join("systemctl");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\necho \"$@\" >> '{}'\n{body}",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    wait_executable(&bin);
    (bin, log)
}

/// Com os testes em paralelo, um `fork` de outra thread pode herdar por um
/// instante o fd de escrita do script, e o `exec` dá `ETXTBSY`. Espera o script
/// poder rodar antes de entregá-lo ao agendador.
fn wait_executable(bin: &Path) {
    for _ in 0..200 {
        match Command::new(bin).arg("__probe").status() {
            Ok(_) => return,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => panic!("o systemctl falso não roda: {e}"),
        }
    }
    panic!("o systemctl falso continua ocupado");
}

fn fake_systemctl(dir: &Path) -> (PathBuf, PathBuf) {
    script(dir, "")
}

fn env_in(dir: &Path) -> Env {
    Env {
        home: Some(dir.into()),
        xdg_config_home: Some(dir.join("config")),
        xdg_runtime_dir: Some(dir.join("run")),
        ..Env::default()
    }
}

const UNIT: &str = "lucerna-routine-lembrete.timer";

#[test]
fn persistent_schedule_writes_config_unit_and_enables() {
    let t = tempfile::tempdir().unwrap();
    let (bin, log) = fake_systemctl(t.path());
    let s = SystemdScheduler::new("routine", &env_in(t.path()))
        .unwrap()
        .with_systemctl(bin);
    s.schedule(&spec("lembrete", now() + 3600, true)).unwrap();
    assert!(t.path().join("config/systemd/user").join(UNIT).exists());
    assert!(!t.path().join("run/systemd/user").join(UNIT).exists());
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!("--user daemon-reload\n--user enable {UNIT}\n--user restart {UNIT}\n")
    );
}

#[test]
fn runtime_schedule_and_cancel() {
    let t = tempfile::tempdir().unwrap();
    let (bin, log) = fake_systemctl(t.path());
    let s = SystemdScheduler::new("routine", &env_in(t.path()))
        .unwrap()
        .with_systemctl(bin);
    s.schedule(&spec("lembrete", now() + 3600, false)).unwrap();
    let file = t.path().join("run/systemd/user").join(UNIT);
    assert!(file.exists());
    assert!(!t.path().join("config/systemd/user").join(UNIT).exists());
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("Persistent")
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!("--user daemon-reload\n--user restart {UNIT}\n")
    );
    s.cancel("lembrete").unwrap();
    assert!(!file.exists());
    let l = std::fs::read_to_string(&log).unwrap();
    assert!(
        l.ends_with(&format!(
            "--user disable --now {UNIT}\n--user daemon-reload\n"
        )),
        "{l}"
    );
}

#[test]
fn rescheduling_changes_the_time() {
    let t = tempfile::tempdir().unwrap();
    let (bin, log) = fake_systemctl(t.path());
    let s = SystemdScheduler::new("routine", &env_in(t.path()))
        .unwrap()
        .with_systemctl(bin);
    let n = now();
    s.schedule(&spec("lembrete", n + 3600, true)).unwrap();
    s.schedule(&spec("lembrete", n + 7200, true)).unwrap();
    let unit = std::fs::read_to_string(t.path().join("config/systemd/user").join(UNIT)).unwrap();
    assert!(unit.contains(&format!("OnCalendar={}\n", calendar_utc(n + 7200))));
    assert!(!unit.contains(&calendar_utc(n + 3600)));
    let once = format!("--user daemon-reload\n--user enable {UNIT}\n--user restart {UNIT}\n");
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!("{once}{once}")
    );
}

#[test]
fn refuses_past_times_and_bad_names() {
    let t = tempfile::tempdir().unwrap();
    let (bin, log) = fake_systemctl(t.path());
    let s = SystemdScheduler::new("routine", &env_in(t.path()))
        .unwrap()
        .with_systemctl(bin);
    assert!(matches!(
        s.schedule(&spec("lembrete", 1, true)),
        Err(ScheduleError::InPast)
    ));
    assert!(matches!(
        s.schedule(&spec("lembrete", now(), false)),
        Err(ScheduleError::InPast)
    ));
    for bad in ["Lembrete", "a/b", "", "-a", &"a".repeat(33)] {
        assert!(
            matches!(
                s.schedule(&spec(bad, now() + 3600, true)),
                Err(ScheduleError::InvalidName(_))
            ),
            "{bad:?}"
        );
        assert!(matches!(s.cancel(bad), Err(ScheduleError::InvalidName(_))));
    }
    assert!(!log.exists());
    assert!(!t.path().join("config/systemd").exists());
    assert!(!t.path().join("run/systemd").exists());
    // App inválido não monta o agendador.
    assert!(matches!(
        SystemdScheduler::new("Routine", &env_in(t.path())),
        Err(ScheduleError::NoDir(_))
    ));
}

#[test]
fn missing_runtime_dir_is_reported() {
    let t = tempfile::tempdir().unwrap();
    let (bin, _log) = fake_systemctl(t.path());
    let env = Env {
        xdg_runtime_dir: None,
        ..env_in(t.path())
    };
    let s = SystemdScheduler::new("routine", &env)
        .unwrap()
        .with_systemctl(bin);
    assert!(matches!(
        s.schedule(&spec("lembrete", now() + 3600, false)),
        Err(ScheduleError::NoDir(_))
    ));
}

#[test]
fn systemctl_failure_is_reported() {
    let t = tempfile::tempdir().unwrap();
    let (bin, _log) = script(t.path(), "echo falhou >&2\nexit 3\n");
    let s = SystemdScheduler::new("routine", &env_in(t.path()))
        .unwrap()
        .with_systemctl(bin);
    match s.schedule(&spec("lembrete", now() + 3600, true)) {
        Err(ScheduleError::Systemctl {
            args,
            status,
            stderr,
        }) => {
            assert_eq!(status, Some(3));
            assert_eq!(args, "--user daemon-reload");
            assert!(stderr.contains("falhou"), "{stderr}");
        }
        other => panic!("esperava Systemctl, veio {other:?}"),
    }
}

#[test]
fn fake_scheduler_records() {
    let f = FakeScheduler::default();
    f.schedule(&spec("lembrete", 10, true)).unwrap();
    f.schedule(&spec("lembrete", 20, false)).unwrap();
    let timers = f.timers();
    assert_eq!(timers.len(), 1);
    assert_eq!(timers["lembrete"], spec("lembrete", 20, false));
    f.cancel("lembrete").unwrap();
    assert!(f.timers().is_empty());
    // Pelo trait, como o app usaria.
    let dyn_s: &dyn Scheduler = &f;
    dyn_s.schedule(&spec("outro", 5, true)).unwrap();
    assert_eq!(f.timers().len(), 1);
}

fn list_timers() -> String {
    let out = Command::new("systemctl")
        .args(["--user", "list-timers", "--all", "--no-pager"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Limpa o que o teste real deixou no systemd, mesmo se ele falhar no meio.
struct RealCleanup {
    scheduler: SystemdScheduler,
    service: PathBuf,
}

impl Drop for RealCleanup {
    fn drop(&mut self) {
        let _ = self.scheduler.cancel("teste");
        let _ = std::fs::remove_file(&self.service);
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        let _ = Command::new("systemctl")
            .args(["--user", "reset-failed", "lucerna-sdktest-teste.timer"])
            .status();
    }
}

#[test]
#[ignore = "precisa de um systemd --user de verdade"]
fn real_systemd_round_trip() {
    let env = Env::from_process();
    let s = SystemdScheduler::new("sdktest", &env).unwrap();
    let unit = "lucerna-sdktest-teste.timer";
    // O systemd recusa ligar um timer cujo serviço não está carregado: um
    // `lucerna-sdktest.service` de mentira, só nesta sessão.
    let dir = env.xdg_runtime_dir.clone().unwrap().join("systemd/user");
    std::fs::create_dir_all(&dir).unwrap();
    let service = dir.join("lucerna-sdktest.service");
    std::fs::write(
        &service,
        "[Unit]\nDescription=Lucerna: teste do SDK\n\n[Service]\nType=oneshot\nExecStart=/bin/true\n",
    )
    .unwrap();
    let cleanup = RealCleanup {
        scheduler: s.clone(),
        service,
    };
    s.schedule(&spec("teste", now() + 3600, false)).unwrap();
    let listed = list_timers();
    assert!(listed.contains(unit), "{listed}");
    s.cancel("teste").unwrap();
    let after = list_timers();
    assert!(!after.contains(unit), "{after}");
    drop(cleanup);
    assert!(!dir.join(unit).exists());
}
