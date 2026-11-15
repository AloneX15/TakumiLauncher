//! Builds the Java command line and runs the game process.

use crate::accounts::{self, LaunchAuth};
use crate::install::{self, Prepared};
use crate::private_instances::{self, LiveOutcome, Synced};
use crate::private_runtime::{self as runtime, MaterializeError, Materialized};
use crate::session;
use crate::settings::{Settings, MIN_MEMORY_MB};
use crate::state::AppState;
use anyhow::{bail, Context};
use download_manager::DownloadProgress;
use minecraft_resolver::{build_arguments, Env};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;
use types::Instance;
use uuid::Uuid;

pub const CANCELLED: &str = "Cancelado";

/// Receives launch progress and game output.
pub trait LaunchSink: Send + Sync {
    fn status(&self, stage: &str, progress: Option<DownloadProgress>);
    fn log(&self, line: String);
    /// A private instance published a version that needs a restart while the game runs.
    fn private_update(&self, _payload: serde_json::Value) {}
}

/// Seconds between checks for a newer published version while a private instance runs.
const LIVE_POLL_SECS: u64 = 30;
/// A game that has not closed itself this long after a restart request is closed by the launcher.
const RESTART_KILL_AFTER_SECS: u64 = 20;

enum Exit {
    Code(Option<i32>),
    /// The game closed to update: start it again.
    Restart,
}

/// Re-check every file the instance uses against its published hash and
/// download again whatever is missing, corrupted or modified.
pub async fn repair(state: &AppState, id: Uuid, sink: &dyn LaunchSink) -> anyhow::Result<()> {
    let instance = state.instances.get(id)?;
    let settings = Settings::load(&state.root);
    install::prepare(state, &instance, &settings, sink, true).await?;
    Ok(())
}

/// Prepare and run an instance until the game exits or `stop` fires.
/// Returns the game's exit code (`None` if it was stopped or killed).
/// A private instance that closes to apply a mandatory update starts again, updated.
pub async fn run(
    state: &AppState,
    id: Uuid,
    mut stop: oneshot::Receiver<()>,
    sink: &dyn LaunchSink,
) -> anyhow::Result<Option<i32>> {
    loop {
        match run_once(state, id, &mut stop, sink).await? {
            Exit::Code(code) => return Ok(code),
            Exit::Restart => sink.log("[Takumi] Reiniciando para aplicar la actualización obligatoria…".into()),
        }
    }
}

async fn run_once(
    state: &AppState,
    id: Uuid,
    stop: &mut oneshot::Receiver<()>,
    sink: &dyn LaunchSink,
) -> anyhow::Result<Exit> {
    let mut instance = state.instances.get(id)?;
    let settings = Settings::load(&state.root);

    // Private instances always update to the published version first: there is no playing an old one.
    let private: Option<Synced> = if instance.managed.is_some() {
        let synced = tokio::select! {
            result = private_instances::sync(state, id, sink) => result?,
            _ = &mut *stop => bail!(CANCELLED),
        };
        instance = synced.instance.clone();
        Some(synced)
    } else {
        None
    };

    let prepare = async {
        sink.status("auth", None);
        let auth = accounts::launch_auth(state, &settings).await?;
        let prepared = install::prepare(state, &instance, &settings, sink, false).await?;
        anyhow::Ok((auth, prepared))
    };
    let (auth, prepared) = tokio::select! {
        result = prepare => result?,
        _ = &mut *stop => bail!(CANCELLED),
    };

    // Private instances: a join ticket for the event server, and the event files decrypted for this run.
    let mut ticket = None;
    let mut materialized = None;
    let mut mark = String::new();
    if let Some(synced) = &private {
        let token = session::api_token(state)?;
        let version = synced.manifest.version;
        let request = runtime::request_ticket(state, id, &token, &auth.uuid, &auth.username, version);
        ticket = Some(tokio::select! {
            result = request => result.map_err(|e| e.context("No se pudo obtener el pase de entrada del evento"))?,
            _ = &mut *stop => bail!(CANCELLED),
        });
        runtime::ensure_safe_signal_dir(&prepared.game_dir)?;
        runtime::clear_signals(&prepared.game_dir);
        let username = session::TakumiSession::load(&state.root)
            .map(|s| s.username)
            .unwrap_or_else(|| "desconocido".into());
        mark = runtime::watermark(&username, &instance.name, id);
        materialized = Some(tokio::select! {
            result = materialize_with_retry(state, id, synced, &mark, sink) => result?,
            _ = &mut *stop => bail!(CANCELLED),
        });
    }

    sink.status("starting", None);
    let mut command = Command::new(&prepared.java);
    command
        .args(launch_arguments(&instance, &settings, &auth, &prepared)?)
        .current_dir(&prepared.game_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let (Some(ticket), Some(synced)) = (&ticket, &private) {
        // Read by the Takumi Gate mod; never put on the command line, where other programs could see it.
        command
            .env("TAKUMI_GATE_TICKET", ticket)
            .env("TAKUMI_INSTANCE_VERSION", synced.manifest.version.to_string());
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
        // Keeps background programs from stealing CPU time from the game.
        let priority = if settings.optimize_game {
            ABOVE_NORMAL_PRIORITY_CLASS
        } else {
            0
        };
        command.creation_flags(CREATE_NO_WINDOW | priority);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("No se pudo iniciar Java ({})", prepared.java.display()))?;

    instance.last_played = Some(chrono::Utc::now());
    if let Err(error) = state.instances.save(&mut instance) {
        tracing::warn!("no se pudo guardar la instancia: {error}");
    }
    sink.status("running", None);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let output = async {
        tokio::join!(
            pump_output(stdout, &auth.access_token, sink),
            pump_output(stderr, &auth.access_token, sink)
        );
    };
    tokio::pin!(output);
    let mut output_done = false;

    let (code, stopped) = {
        let live = async {
            match (&private, &materialized) {
                (Some(synced), Some(running)) => {
                    watch_live(state, id, synced, running, &mark, &prepared.game_dir, sink).await
                }
                _ => std::future::pending().await,
            }
        };
        tokio::pin!(live);

        let mut stopped = false;
        let code = loop {
            tokio::select! {
                status = child.wait() => break status?.code(),
                _ = &mut output, if !output_done => output_done = true,
                _ = &mut live => {
                    // The player asked to restart and the game did not close by itself.
                    child.kill().await?;
                    break None;
                }
                _ = &mut *stop => {
                    child.kill().await?;
                    stopped = true;
                    break None;
                }
            }
        };
        (code, stopped)
    };
    if !output_done {
        // Flush what the game wrote just before exiting.
        let _ = tokio::time::timeout(Duration::from_secs(2), output).await;
    }
    // Event files leave the game folder as soon as the game is gone.
    drop(materialized);
    if private.is_some() {
        let restart = runtime::take_restart_request(&prepared.game_dir);
        runtime::clear_signals(&prepared.game_dir);
        if restart && !stopped {
            return Ok(Exit::Restart);
        }
    }
    Ok(Exit::Code(code))
}

/// Decrypts the event files; objects that fail (modified or from an old key) are downloaded again once.
async fn materialize_with_retry(
    state: &AppState,
    id: Uuid,
    synced: &Synced,
    mark: &str,
    sink: &dyn LaunchSink,
) -> anyhow::Result<Materialized> {
    let instance_dir = state.instances.instance_dir(id);
    let game_dir = state.instances.game_dir(id);
    for attempt in 0..2 {
        let (dir, game, files, key, mark) = (
            instance_dir.clone(),
            game_dir.clone(),
            synced.manifest.files.clone(),
            synced.key.clone(),
            mark.to_string(),
        );
        let result =
            tokio::task::spawn_blocking(move || runtime::materialize(&dir, &game, &files, &key, &mark)).await?;
        match result {
            Ok(materialized) => return Ok(materialized),
            Err(MaterializeError::Corrupt(list)) if attempt == 0 => {
                sink.log(format!(
                    "[Takumi] {} archivos de la instancia estaban dañados; se descargan de nuevo.",
                    list.len()
                ));
                let token = session::api_token(state)?;
                private_instances::download_and_seal(
                    state,
                    &synced.manifest.files,
                    &instance_dir,
                    &game_dir,
                    id,
                    &token,
                    &synced.key,
                    sink,
                )
                .await?;
            }
            Err(MaterializeError::Corrupt(_)) => {
                bail!("Los archivos de la instancia siguen dañados tras descargarlos de nuevo.")
            }
            Err(MaterializeError::Other(error)) => return Err(error),
        }
    }
    unreachable!("the loop returns on its second attempt")
}

/// Runs next to a private instance's game: applies resource-pack-only versions live, tells the game and
/// the launcher window when a version needs a restart, and returns when a requested restart is overdue
/// (the game did not close by itself).
async fn watch_live(
    state: &AppState,
    id: Uuid,
    synced: &Synced,
    running: &Materialized,
    mark: &str,
    game_dir: &Path,
    sink: &dyn LaunchSink,
) {
    let mut installed = synced.manifest.version;
    let mut notified = installed;
    let mut seconds = 0u64;
    let mut locked = false;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        seconds += 1;
        if runtime::restart_request_age(game_dir).is_some_and(|age| age >= RESTART_KILL_AFTER_SECS) {
            return;
        }
        // The Takumi Gate mod says every jar is open: nobody else may read them while the game runs.
        if !locked && runtime::signal_path(game_dir, runtime::LOADED_FILE).is_file() {
            locked = true;
            match runtime::lock_running(game_dir) {
                Ok(_) => sink.log("[Takumi] Carpeta de mods bloqueada mientras juegas.".into()),
                Err(error) => tracing::warn!("no se pudo bloquear la carpeta de mods: {error:#}"),
            }
        }
        if seconds == 300 && !locked {
            sink.log(
                "[Takumi] Esta instancia no incluye Takumi Gate: la carpeta de mods no se puede bloquear mientras juegas."
                    .into(),
            );
        }
        if !seconds.is_multiple_of(LIVE_POLL_SECS) {
            continue;
        }
        let Ok(token) = session::api_token(state) else { continue };
        let status = match runtime::fetch_status(state, id, &token, installed).await {
            Ok(status) => status,
            Err(error) => {
                tracing::debug!("estado de la instancia privada no disponible: {error:#}");
                continue;
            }
        };
        if status.version <= notified {
            continue;
        }
        notified = status.version;

        if !status.restart_required {
            match private_instances::live_apply(state, id, &token, &synced.key, mark, running, sink).await {
                Ok((version, LiveOutcome::Applied { packs })) => {
                    installed = version;
                    let signal = serde_json::json!({ "version": version, "packs": packs });
                    if let Err(error) = runtime::write_signal(game_dir, runtime::RELOAD_FILE, &signal) {
                        tracing::warn!("no se pudo avisar al juego: {error:#}");
                    }
                    sink.log(format!(
                        "[Takumi] Versión {version} aplicada en directo (packs de recursos)."
                    ));
                    continue;
                }
                Ok((_, LiveOutcome::NeedsRestart)) => {}
                Err(error) => sink.log(format!("[Takumi] No se pudo aplicar la versión en directo: {error:#}")),
            }
        }

        let deadline = chrono::Utc::now() + chrono::Duration::minutes(i64::from(status.grace_minutes));
        let notice = serde_json::json!({
            "version": status.version,
            "grace_minutes": status.grace_minutes,
            "deadline": deadline.to_rfc3339(),
            "notes": status.notes,
        });
        if let Err(error) = runtime::write_signal(game_dir, runtime::NOTICE_FILE, &notice) {
            tracing::warn!("no se pudo avisar al juego: {error:#}");
        }
        sink.private_update(notice);
        sink.log(format!(
            "[Takumi] Hay una actualización obligatoria (versión {}). Reinicia el juego para evitar errores.",
            status.version
        ));
    }
}

/// Full argument list for the Java executable.
fn launch_arguments(
    instance: &Instance,
    settings: &Settings,
    auth: &LaunchAuth,
    prepared: &Prepared,
) -> anyhow::Result<Vec<String>> {
    let info = &prepared.info;
    let main_class = info
        .main_class
        .clone()
        .context("La versión no declara clase principal.")?;
    let separator = if cfg!(windows) { ";" } else { ":" };
    let classpath = prepared
        .classpath
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(separator);
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();

    let mut vars: HashMap<String, String> = [
        ("auth_player_name", auth.username.clone()),
        ("auth_uuid", auth.uuid.clone()),
        ("auth_access_token", auth.access_token.clone()),
        ("auth_session", auth.access_token.clone()),
        ("auth_xuid", String::new()),
        ("clientid", String::new()),
        ("user_type", auth.user_type.to_string()),
        ("user_properties", "{}".to_string()),
        ("version_name", info.id.clone()),
        (
            "version_type",
            info.version_type.clone().unwrap_or_else(|| "release".into()),
        ),
        ("game_directory", path(&prepared.game_dir)),
        ("assets_root", path(&prepared.assets_dir)),
        ("game_assets", path(&prepared.game_assets_dir)),
        (
            "assets_index_name",
            info.asset_index.as_ref().map(|a| a.id.clone()).unwrap_or_default(),
        ),
        ("natives_directory", path(&prepared.natives_dir)),
        ("library_directory", path(&prepared.libraries_dir)),
        ("classpath", classpath),
        ("classpath_separator", separator.to_string()),
        ("launcher_name", "TakumiLauncher".to_string()),
        ("launcher_version", env!("CARGO_PKG_VERSION").to_string()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();

    let mut env = Env::current();
    // Private instances join their event server directly (quick play, 1.20+).
    let server = instance.managed.as_ref().and_then(|m| m.server.clone());
    if let Some(server) = &server {
        env = env.with_feature("is_quick_play_multiplayer", true);
        vars.insert("quickPlayMultiplayer".into(), server.clone());
    }
    let resolution = instance.settings.width.zip(instance.settings.height);
    if let Some((width, height)) = resolution {
        env = env.with_feature("has_custom_resolution", true);
        vars.insert("resolution_width".into(), width.to_string());
        vars.insert("resolution_height".into(), height.to_string());
    }
    let arguments = build_arguments(info, &env, &vars);

    let memory = instance
        .settings
        .memory_mb
        .unwrap_or(settings.memory_mb)
        .max(MIN_MEMORY_MB);
    let extra_jvm = instance
        .settings
        .jvm_args
        .as_deref()
        .filter(|args| !args.trim().is_empty())
        .unwrap_or(&settings.jvm_args);

    let mut args = vec![format!("-Xmx{memory}M")];
    if settings.optimize_game {
        args.extend(performance_jvm_args(memory, extra_jvm));
    }
    // After the tuning, so the user's own arguments win.
    args.extend(extra_jvm.split_whitespace().map(str::to_string));
    // Defence in depth against Log4Shell on 1.17–1.18; older versions rely
    // on Mojang's patched logging configuration below.
    args.push("-Dlog4j2.formatMsgNoLookups=true".to_string());
    args.extend(prepared.logging_argument.clone());
    args.extend(arguments.jvm);
    args.push(main_class);
    let quick_play = arguments.game.iter().any(|arg| arg == "--quickPlayMultiplayer");
    args.extend(arguments.game);
    if let (Some(server), false) = (&server, quick_play) {
        // Versions before quick play take the older --server/--port pair.
        let (host, port) = match server.rsplit_once(':') {
            Some((host, port)) if port.parse::<u16>().is_ok() => (host.to_string(), port.to_string()),
            _ => (server.clone(), "25565".to_string()),
        };
        args.extend(["--server".into(), host, "--port".into(), port]);
    }
    if let (Some((width, height)), true) = (resolution, info.minecraft_arguments.is_some()) {
        // Legacy argument strings have no resolution placeholders.
        args.extend([
            "--width".into(),
            width.to_string(),
            "--height".into(),
            height.to_string(),
        ]);
    }
    Ok(args)
}

/// JVM tuning for the game client, skipping whatever the user set themselves.
///
/// The garbage collector settings are the ones the official launcher uses:
/// G1 with a larger young generation and big regions, which keeps collections
/// short and avoids "humongous" allocations for chunk and texture data.
fn performance_jvm_args(memory_mb: u32, user_args: &str) -> Vec<String> {
    let user: Vec<&str> = user_args.split_whitespace().collect();
    let mut args = Vec::new();

    if !user.iter().any(|arg| arg.starts_with("-Xms")) {
        // Commit half the heap up front (at least 1 GiB) instead of growing it
        // from the JVM default through a series of early collections.
        let initial = (memory_mb / 2).max(1024).min(memory_mb);
        args.push(format!("-Xms{initial}M"));
    }

    let picks_collector = user
        .iter()
        .any(|arg| arg.starts_with("-XX:+Use") && arg.ends_with("GC"));
    if !picks_collector {
        args.extend(
            [
                "-XX:+UnlockExperimentalVMOptions",
                "-XX:+UseG1GC",
                "-XX:G1NewSizePercent=20",
                "-XX:G1ReservePercent=20",
                "-XX:MaxGCPauseMillis=50",
            ]
            .map(str::to_string),
        );
        // Regions must stay small relative to the heap: G1 needs plenty of them.
        let region_mb = match memory_mb {
            4096.. => Some(32),
            2048.. => Some(16),
            1024.. => Some(8),
            _ => None,
        };
        if let Some(region_mb) = region_mb {
            args.push(format!("-XX:G1HeapRegionSize={region_mb}M"));
        }
    }
    args
}

async fn pump_output<R: AsyncRead + Unpin>(reader: Option<R>, secret: &str, sink: &dyn LaunchSink) {
    let Some(reader) = reader else { return };
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    let mut cleaner = LogCleaner::default();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buffer);
                if let Some(mut line) = cleaner.clean(line.trim_end()) {
                    // Old versions print the session token on startup.
                    if secret.len() > 8 && line.contains(secret) {
                        line = line.replace(secret, "****");
                    }
                    sink.log(line);
                }
            }
        }
    }
}

/// Turns the log4j XML events emitted under Mojang's logging configuration
/// back into plain lines; other output passes through unchanged.
#[derive(Default)]
struct LogCleaner {
    /// Level of the event being read, until its first line is emitted.
    pending_level: Option<String>,
}

impl LogCleaner {
    fn clean(&mut self, line: &str) -> Option<String> {
        let trimmed = line.trim_start();
        if trimmed.starts_with("<log4j:Event") {
            self.pending_level = trimmed
                .split("level=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string);
            return None;
        }
        if trimmed.starts_with("</log4j:Event>") {
            self.pending_level = None;
            return None;
        }
        if !trimmed.contains("log4j:") {
            return Some(line.to_string());
        }

        let mut text = trimmed.to_string();
        for tag in [
            "<log4j:Message><![CDATA[",
            "]]></log4j:Message>",
            "<log4j:Throwable><![CDATA[",
            "]]></log4j:Throwable>",
        ] {
            text = text.replace(tag, "");
        }
        if text.is_empty() {
            return None;
        }
        Some(match self.pending_level.take() {
            Some(level) => format!("[{level}] {text}"),
            None => text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minecraft_resolver::VersionInfo;
    use std::path::PathBuf;
    use types::{InstanceSettings, ModLoader};

    #[test]
    fn log_cleaner_unwraps_xml_events() {
        let mut cleaner = LogCleaner::default();
        let input = [
            r#"<log4j:Event logger="a.b" timestamp="1" level="INFO" thread="main">"#,
            "  <log4j:Message><![CDATA[Setting user: Steve]]></log4j:Message>",
            "</log4j:Event>",
            "plain line",
            r#"<log4j:Event logger="a.b" timestamp="2" level="ERROR" thread="main">"#,
            "  <log4j:Message><![CDATA[first",
            "second]]></log4j:Message>",
            "</log4j:Event>",
        ];
        let output: Vec<String> = input.iter().filter_map(|line| cleaner.clean(line)).collect();
        assert_eq!(
            output,
            vec!["[INFO] Setting user: Steve", "plain line", "[ERROR] first", "second"]
        );
    }

    fn sample() -> (Instance, Settings, LaunchAuth, Prepared) {
        let info: VersionInfo = serde_json::from_str(
            r#"{
                "id": "1.21.1", "type": "release",
                "mainClass": "net.minecraft.client.main.Main",
                "assetIndex": {"id": "17", "url": "https://x"},
                "arguments": {
                    "game": ["--username", "${auth_player_name}", "--uuid", "${auth_uuid}",
                             "--accessToken", "${auth_access_token}", "--gameDir", "${game_directory}",
                             "--assetIndex", "${assets_index_name}", "--userType", "${user_type}",
                             {"rules": [{"action": "allow", "features": {"has_custom_resolution": true}}],
                              "value": ["--width", "${resolution_width}", "--height", "${resolution_height}"]}],
                    "jvm": ["-Djava.library.path=${natives_directory}",
                            "-Dminecraft.launcher.brand=${launcher_name}",
                            "-cp", "${classpath}"]
                }
            }"#,
        )
        .unwrap();
        let now = chrono::Utc::now();
        let instance = Instance {
            id: Uuid::new_v4(),
            name: "Test".into(),
            description: None,
            minecraft_version: "1.21.1".into(),
            mod_loader: ModLoader::Vanilla,
            mod_loader_version: None,
            created_at: now,
            updated_at: now,
            last_played: None,
            settings: InstanceSettings::default(),
            managed: None,
        };
        let auth = LaunchAuth {
            username: "Steve".into(),
            uuid: "00000000000000000000000000000001".into(),
            access_token: "token-value".into(),
            user_type: "msa",
        };
        let prepared = Prepared {
            info,
            classpath: vec![PathBuf::from("libs/a.jar"), PathBuf::from("versions/client.jar")],
            java: PathBuf::from("java"),
            game_dir: PathBuf::from("game"),
            natives_dir: PathBuf::from("natives"),
            assets_dir: PathBuf::from("root/assets"),
            libraries_dir: PathBuf::from("root/libraries"),
            game_assets_dir: PathBuf::from("root/assets/virtual/17"),
            logging_argument: Some("-Dlog4j.configurationFile=client.xml".into()),
        };
        (instance, Settings::default(), auth, prepared)
    }

    #[test]
    fn arguments_are_ordered_and_fully_substituted() {
        let (instance, settings, auth, prepared) = sample();
        let args = launch_arguments(&instance, &settings, &auth, &prepared).unwrap();

        assert_eq!(args[0], "-Xmx4096M");
        assert_eq!(args[1], "-Xms2048M");
        assert!(args.contains(&"-XX:+UseG1GC".to_string()));
        assert!(args.contains(&"-XX:G1HeapRegionSize=32M".to_string()));
        assert!(args.iter().all(|arg| !arg.contains("${")), "{args:?}");

        let main = args.iter().position(|a| a == "net.minecraft.client.main.Main").unwrap();
        let classpath = args.iter().position(|a| a == "-cp").unwrap();
        let separator = if cfg!(windows) { ";" } else { ":" };
        assert!(classpath < main);
        assert_eq!(args[classpath + 1], format!("libs/a.jar{separator}versions/client.jar"));
        assert!(args[..main].contains(&"-Dlog4j.configurationFile=client.xml".to_string()));
        assert!(args[..main].contains(&"-Dminecraft.launcher.brand=TakumiLauncher".to_string()));

        let game = &args[main + 1..];
        assert_eq!(
            &game[..4],
            ["--username", "Steve", "--uuid", "00000000000000000000000000000001"]
        );
        assert!(game.windows(2).any(|w| w == ["--userType", "msa"]));
        assert!(!game.contains(&"--width".to_string()));
    }

    #[test]
    fn instance_overrides_apply() {
        let (mut instance, mut settings, auth, prepared) = sample();
        settings.jvm_args = "-XX:+UseG1GC".into();
        instance.settings = InstanceSettings {
            memory_mb: Some(8192),
            jvm_args: Some("-XX:+UseZGC -Dfoo=bar".into()),
            width: Some(1280),
            height: Some(720),
            java_path: None,
        };
        let args = launch_arguments(&instance, &settings, &auth, &prepared).unwrap();
        // The user chose a collector: only the initial heap is added.
        assert_eq!(&args[..4], ["-Xmx8192M", "-Xms4096M", "-XX:+UseZGC", "-Dfoo=bar"]);
        assert!(!args.contains(&"-XX:+UseG1GC".to_string()));
        assert!(args.windows(4).any(|w| w == ["--width", "1280", "--height", "720"]));
    }

    #[test]
    fn performance_arguments_respect_user_choices() {
        assert_eq!(performance_jvm_args(512, "")[0], "-Xms512M");
        assert_eq!(performance_jvm_args(2048, "")[0], "-Xms1024M");
        assert!(performance_jvm_args(2048, "").contains(&"-XX:G1HeapRegionSize=16M".to_string()));
        assert!(!performance_jvm_args(512, "")
            .iter()
            .any(|a| a.contains("G1HeapRegionSize")));
        assert!(performance_jvm_args(4096, "-Xms512M -XX:+UseShenandoahGC").is_empty());

        let (instance, mut settings, auth, prepared) = sample();
        settings.optimize_game = false;
        let args = launch_arguments(&instance, &settings, &auth, &prepared).unwrap();
        assert!(!args.iter().any(|a| a.starts_with("-Xms") || a.contains("G1")));
    }
}
