//! Launch benchmarks against the launcher's real data directory, using
//! instances that are already installed (nothing is downloaded when they are
//! up to date).
//!
//! ```text
//! # Time from pressing "Jugar" to the Java process being ready to start
//! cargo test -p takumi-launcher bench_prepare -- --ignored --nocapture
//!
//! # Also start the game (opens a window) and time it until it has booted
//! TAKUMI_BENCH_INSTANCE="Fabric" cargo test -p takumi-launcher bench_boot -- --ignored --nocapture
//! ```
//!
//! `TAKUMI_BENCH_ROOT` overrides the data directory, `TAKUMI_BENCH_RUNS` the
//! number of repetitions and `TAKUMI_BENCH_JVM` sets the instance's extra JVM
//! arguments for the benchmark only (e.g. `-Xlog:gc:file=gc.log`).

use crate::install;
use crate::launch::{self, LaunchSink};
use crate::settings::Settings;
use crate::state::AppState;
use download_manager::DownloadProgress;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use types::Instance;

/// Printed once the client has loaded its resources and reached the menu.
const BOOT_MARKERS: &[&str] = &["Sound engine started", "OpenAL initialized"];

fn root() -> PathBuf {
    std::env::var("TAKUMI_BENCH_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("APPDATA").expect("APPDATA")).join("com.takumilabs.launcher"))
}

fn runs() -> usize {
    std::env::var("TAKUMI_BENCH_RUNS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(3)
}

fn selected(state: &AppState) -> Vec<Instance> {
    let filter = std::env::var("TAKUMI_BENCH_INSTANCE")
        .unwrap_or_default()
        .to_lowercase();
    let mut instances = state.instances.list().unwrap();
    instances.retain(|i| i.name.to_lowercase().contains(&filter));
    assert!(!instances.is_empty(), "no instance matches TAKUMI_BENCH_INSTANCE");
    instances
}

/// Puts the instance back as it was (JVM arguments, "last played"), even if the benchmark panics.
struct Restore<'a> {
    state: &'a AppState,
    original: Instance,
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        let _ = self.state.instances.save(&mut self.original);
    }
}

/// Records when each stage starts and forwards boot markers.
struct Timeline {
    stages: Mutex<Vec<(String, Instant)>>,
    booted: Option<mpsc::UnboundedSender<Instant>>,
}

impl Timeline {
    fn new(booted: Option<mpsc::UnboundedSender<Instant>>) -> Self {
        Self {
            stages: Mutex::new(Vec::new()),
            booted,
        }
    }

    fn started(&self, stage: &str) -> Option<Instant> {
        let stages = self.stages.lock().unwrap();
        stages.iter().find(|(name, _)| name == stage).map(|(_, at)| *at)
    }

    fn report(&self, end: Instant) -> String {
        let stages = self.stages.lock().unwrap();
        let mut parts = Vec::new();
        for (index, (name, at)) in stages.iter().enumerate() {
            let next = stages.get(index + 1).map(|(_, at)| *at).unwrap_or(end);
            parts.push(format!("{name} {} ms", next.duration_since(*at).as_millis()));
        }
        parts.join(" | ")
    }
}

impl LaunchSink for Timeline {
    fn status(&self, stage: &str, _progress: Option<DownloadProgress>) {
        let mut stages = self.stages.lock().unwrap();
        if stages.last().map(|(name, _)| name.as_str()) != Some(stage) {
            stages.push((stage.to_string(), Instant::now()));
        }
    }

    fn log(&self, line: String) {
        if let Some(booted) = &self.booted {
            if BOOT_MARKERS.iter().any(|marker| line.contains(marker)) {
                let _ = booted.send(Instant::now());
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses the real launcher data directory"]
async fn bench_prepare() {
    let state = AppState::new(root()).unwrap();
    let settings = Settings::load(&state.root);
    for instance in selected(&state) {
        for run in 1..=runs() {
            let sink = Timeline::new(None);
            let start = Instant::now();
            install::prepare(&state, &instance, &settings, &sink, false)
                .await
                .unwrap();
            let end = Instant::now();
            println!(
                "[prepare] {} #{run}: {} ms  ({})",
                instance.name,
                end.duration_since(start).as_millis(),
                sink.report(end)
            );
        }
    }
}

/// Start the instance, wait until it has booted and stop it.
/// Returns (time until Java started, time from then until the menu).
async fn boot(state: &AppState, id: uuid::Uuid) -> (Duration, Duration) {
    let (booted_tx, mut booted_rx) = mpsc::unbounded_channel();
    let sink = Timeline::new(Some(booted_tx));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let start = Instant::now();
    let game = launch::run(state, id, stop_rx, &sink);
    tokio::pin!(game);

    let booted = tokio::select! {
        result = &mut game => panic!("the game exited before booting: {result:?}"),
        at = booted_rx.recv() => at.unwrap(),
        _ = tokio::time::sleep(Duration::from_secs(600)) => panic!("timed out waiting for the game to boot"),
    };
    let spawned = sink.started("running").unwrap();
    stop_tx.send(()).unwrap();
    let _ = game.await;
    // Give the OS a moment to release the game's files before the next run.
    tokio::time::sleep(Duration::from_secs(2)).await;
    (spawned.duration_since(start), booted.duration_since(spawned))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts the game and opens a window"]
async fn bench_boot() {
    let state = AppState::new(root()).unwrap();
    for original in selected(&state) {
        let _restore = Restore {
            state: &state,
            original: original.clone(),
        };
        if let Ok(jvm_args) = std::env::var("TAKUMI_BENCH_JVM") {
            let mut patched = original.clone();
            patched.settings.jvm_args = Some(jvm_args);
            state.instances.save(&mut patched).unwrap();
        }
        for run in 1..=runs() {
            let (prepare, boot) = boot(&state, original.id).await;
            println!(
                "[boot] {} #{run}: jugar->java {} ms, java->menu {} ms",
                original.name,
                prepare.as_millis(),
                boot.as_millis(),
            );
        }
    }
}

/// Installs the performance mods into a throwaway copy of an instance's setup
/// (same game version and loader, no worlds), boots it and deletes it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to Modrinth, starts the game and opens a window"]
async fn bench_optimize() {
    use crate::content;
    use instance_manager::NewInstance;

    let state = AppState::new(root()).unwrap();
    let template = selected(&state).remove(0);
    let instance = state
        .instances
        .create(NewInstance {
            name: "takumi bench optimize".into(),
            minecraft_version: template.minecraft_version.clone(),
            mod_loader: template.mod_loader,
            mod_loader_version: template.mod_loader_version.clone(),
        })
        .unwrap();
    struct Delete<'a>(&'a AppState, uuid::Uuid);
    impl Drop for Delete<'_> {
        fn drop(&mut self) {
            let _ = self.0.instances.delete(self.1);
        }
    }
    let _delete = Delete(&state, instance.id);

    let start = Instant::now();
    let report = content::install_performance_mods(&state, &instance).await.unwrap();
    println!("[optimize] {} ms: {report:?}", start.elapsed().as_millis());
    assert!(!report.installed.is_empty(), "nothing was installed");

    // A second pass recognises everything by file hash and installs nothing.
    let again = content::install_performance_mods(&state, &instance).await.unwrap();
    println!("[optimize] second pass: {again:?}");
    assert!(again.installed.is_empty());

    for run in 1..=runs() {
        let (prepare, boot) = boot(&state, instance.id).await;
        println!(
            "[boot+mods] #{run}: jugar->java {} ms, java->menu {} ms",
            prepare.as_millis(),
            boot.as_millis()
        );
    }
}

/// Puts a file back from its backup, even if the test panics.
struct RestoreFile {
    path: PathBuf,
    backup: Vec<u8>,
}

impl Drop for RestoreFile {
    fn drop(&mut self) {
        if std::fs::read(&self.path).ok().as_deref() != Some(self.backup.as_slice()) {
            std::fs::write(&self.path, &self.backup).expect("could not restore the library");
        }
    }
}

/// Corrupts one library of the first selected instance (same size, different
/// bytes), checks that a normal launch does not notice and that "repair"
/// downloads it again. The original file is restored in any case.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses the real launcher data directory and downloads one library"]
async fn bench_repair() {
    let state = AppState::new(root()).unwrap();
    let settings = Settings::load(&state.root);
    let instance = selected(&state).remove(0);
    let prepared = install::prepare(&state, &instance, &settings, &Timeline::new(None), false)
        .await
        .unwrap();
    let library = prepared
        .classpath
        .iter()
        .find(|path| path.starts_with(&prepared.libraries_dir) && path.extension().is_some_and(|e| e == "jar"))
        .expect("no library on the classpath")
        .clone();
    let original = std::fs::read(&library).unwrap();
    let _restore = RestoreFile {
        path: library.clone(),
        backup: original.clone(),
    };
    let corrupted: Vec<u8> = original.iter().map(|b| b ^ 0x5a).collect();
    std::fs::write(&library, &corrupted).unwrap();
    println!("[repair] {} — corrupted {}", instance.name, library.display());

    install::prepare(&state, &instance, &settings, &Timeline::new(None), false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(&library).unwrap(),
        corrupted,
        "a normal launch is not expected to rehash"
    );

    let start = Instant::now();
    launch::repair(&state, instance.id, &Timeline::new(None)).await.unwrap();
    println!("[repair] verified every file in {} ms", start.elapsed().as_millis());
    assert_eq!(
        std::fs::read(&library).unwrap(),
        original,
        "repair did not restore the library"
    );
}

/// Prints what the optimizer would do for each selected instance; installs nothing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to Modrinth"]
async fn bench_optimize_plan() {
    let state = AppState::new(root()).unwrap();
    for instance in selected(&state) {
        let plan = crate::content::performance_plan(&state, &instance).await.unwrap();
        println!("[plan] {}: {plan:?}", instance.name);
    }
}
