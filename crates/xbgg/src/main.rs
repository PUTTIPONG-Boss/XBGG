#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The GNU toolchain links its own default manifest next to ours and ld warns
// about the duplicate. Ours wins (verified: Windows loads the asInvoker /
// PerMonitorV2 manifest from assets/xbgg.manifest), so the warning is noise.
#![allow(linker_messages)]

mod art;
mod gamepad;
mod platform;
mod update;
#[cfg(windows)]
mod boost;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::{Color, ComponentHandle, Image, ModelRc, Timer, TimerMode, VecModel};
use xbgg_core::{Game, Library, Source, ADDABLE_EXTENSIONS};

use gamepad::PadAction;

slint::include_modules!();

/// Ignore repeated "Play" presses while a launch is already in progress.
const LAUNCH_COOLDOWN: Duration = Duration::from_secs(3);
const STATUS_DURATION: Duration = Duration::from_secs(5);
/// Set on the process started after an update is installed.
const RESTART_ENV: &str = "XBGG_RESTARTED_AFTER_UPDATE";

struct App {
    library: RefCell<Library>,
    model: Rc<VecModel<GameTile>>,
    status_timer: Timer,
    last_launch: Cell<Option<Instant>>,
    picking_file: Cell<bool>,
    hero: art::HeroLoader,
    covers: RefCell<art::CoverCache>,
    layout_timer: Timer,
    /// Filled by the background update check.
    pending_update: Arc<Mutex<Option<update::Release>>>,
    boost: RefCell<boost::Controller>,
    /// Rows currently shown on the "Apps to freeze" screen.
    app_rows: RefCell<Vec<xbgg_core::boost::apps::AppInfo>>,
}

impl App {
    fn load_hero(&self, ui: &AppWindow, index: i32) {
        let path = usize::try_from(index)
            .ok()
            .and_then(|i| self.library.borrow().games().get(i).and_then(|g| g.hero.clone()));
        let size = ui.window().size();
        self.hero.request(ui, path, size.width, size.height);
    }

    /// Tile size in physical pixels, so covers are drawn 1:1.
    fn cover_size(ui: &AppWindow) -> art::CoverSize {
        let scale = ui.window().scale_factor();
        let px = |logical: f32| (logical * scale).round().max(0.0) as u32;
        art::CoverSize {
            width: px(ui.get_tile_width()),
            height: px(ui.get_tile_height()),
            radius: px(ui.get_tile_radius()),
        }
    }

    /// Pushes the library into the UI, optionally focusing the entry with `select_id`.
    fn sync(&self, ui: &AppWindow, select_id: Option<&str>) {
        let lib = self.library.borrow();
        let paths: Vec<_> = lib.games().iter().filter_map(|g| g.cover.clone()).collect();
        let mut covers = self.covers.borrow_mut();
        covers.ensure(&paths, Self::cover_size(ui));
        covers.retain(&paths);
        let tiles: Vec<GameTile> = lib
            .games()
            .iter()
            .map(|g| to_tile(g, g.cover.as_deref().and_then(|p| covers.get(p))))
            .collect();
        drop(covers);
        self.model.set_vec(tiles);
        let count = lib.games().len() as i32;
        ui.set_scanned_count(count);
        let selected = select_id
            .and_then(|id| lib.index_of(id))
            .map(|i| i as i32)
            .unwrap_or_else(|| ui.get_selected().clamp(0, count));
        drop(lib);
        ui.set_selected(selected);
        self.load_hero(ui, selected);
    }

    fn set_status(&self, ui: &AppWindow, text: impl Into<slint::SharedString>) {
        ui.set_status(text.into());
        let weak = ui.as_weak();
        self.status_timer.start(TimerMode::SingleShot, STATUS_DURATION, move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_status("".into());
            }
        });
    }
}

const HELP: &str = "\
XBGG - full-screen console mode for Windows

Usage: xbgg [OPTIONS]

Options:
  --windowed       Run in a window instead of full screen
  --no-update      Skip the update check on startup
  --list           Print the detected game library and exit
  --check-update   Check GitHub for a newer release and exit
  --restore        Unfreeze apps left frozen by an interrupted boost and exit
  --version        Print the version and exit
  --help           Show this help";

/// Release builds abort on panic without a console; keep a record of why.
fn install_crash_log() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = dirs_config_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let line = format!(
                "[{}] xbgg {} crashed: {info}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                update::current_version()
            );
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(line.as_bytes());
            }
        }
        default_hook(info);
    }));
}

fn dirs_config_dir() -> Option<std::path::PathBuf> {
    xbgg_core::store::default_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn main() -> anyhow::Result<()> {
    install_crash_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    // Internal: restores a boost if the XBGG process with this PID dies.
    if let Some(i) = args.iter().position(|a| a == "--boost-guard") {
        if let Some(pid) = args.get(i + 1).and_then(|p| p.parse().ok()) {
            boost::guard(pid);
        }
        return Ok(());
    }
    if has("--restore") {
        platform::attach_console();
        match boost::recover() {
            Some(r) => println!(
                "Restored an interrupted boost: {} processes resumed, {} already gone{}",
                r.resumed,
                r.gone,
                if r.power_restored { ", power mode restored" } else { "" }
            ),
            None => println!("Nothing to restore"),
        }
        return Ok(());
    }
    if has("--help") || has("-h") {
        platform::attach_console();
        println!("{HELP}");
        return Ok(());
    }
    if has("--version") || has("-V") {
        platform::attach_console();
        println!("xbgg {}", update::current_version());
        return Ok(());
    }
    if has("--list") {
        platform::attach_console();
        return list_library();
    }
    if has("--check-update") {
        platform::attach_console();
        return match update::check()? {
            Some(r) => {
                println!("Update available: {} -> {}", update::current_version(), r.version);
                Ok(())
            }
            None => {
                println!("Up to date ({})", update::current_version());
                Ok(())
            }
        };
    }
    if let Some(unknown) = args.iter().find(|a| {
        !matches!(a.as_str(), "--windowed" | "--no-update")
    }) {
        platform::attach_console();
        eprintln!("Unknown option: {unknown}\n\n{HELP}");
        std::process::exit(2);
    }

    // One shell at a time. After an update the new process starts while the
    // old one is still closing, so it waits briefly for the lock.
    let restarting = std::env::var_os(RESTART_ENV).is_some();
    // Don't pass the marker on to games we launch.
    std::env::remove_var(RESTART_ENV);
    let wait = if restarting { Duration::from_secs(15) } else { Duration::ZERO };
    let Some(_instance) = platform::single_instance(wait) else {
        platform::focus_existing_instance();
        return Ok(());
    };

    update::cleanup_previous();
    // A boost interrupted by a crash or power loss: put everything back first.
    let recovered = boost::recover();

    let ui = AppWindow::new()?;
    ui.set_version(update::current_version().to_string().into());
    let model = Rc::new(VecModel::<GameTile>::default());
    ui.set_games(ModelRc::from(model.clone()));

    let app = Rc::new(App {
        library: RefCell::new(Library::load()),
        model,
        status_timer: Timer::default(),
        last_launch: Cell::new(None),
        picking_file: Cell::new(false),
        hero: art::HeroLoader::default(),
        covers: RefCell::default(),
        layout_timer: Timer::default(),
        pending_update: Arc::default(),
        boost: RefCell::default(),
        app_rows: RefCell::default(),
    });
    app.sync(&ui, None);
    let had_warning = report_warnings(&app, &ui);
    if let Some(r) = recovered.filter(|r| r.resumed > 0 || r.power_restored) {
        if !had_warning {
            app.set_status(&ui, format!("Restored an interrupted boost: {} processes resumed", r.resumed));
        }
    }

    wire_callbacks(&app, &ui);
    let _clock = start_clock(&ui);
    start_gamepad(&ui);
    if !has("--no-update") {
        start_update_check(&app, &ui);
    }

    if !has("--windowed") {
        ui.window().set_fullscreen(true);
    }
    // The real window size and DPI are only known once the window is shown;
    // prepare artwork for them then.
    let weak = ui.as_weak();
    Timer::single_shot(Duration::from_millis(100), move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_layout_changed();
        }
    });
    let result = ui.run();
    // However XBGG closes (menu, window button, update restart), unfreeze
    // everything before the process ends.
    app.boost.borrow_mut().shutdown();
    result?;
    Ok(())
}

fn on_off(on: bool) -> slint::SharedString {
    if on { "On".into() } else { "Off".into() }
}

fn refresh_menu(app: &App, ui: &AppWindow) {
    let cfg = app.library.borrow().boost().clone();
    let active = app.boost.borrow_mut().is_active();
    let row = |id: &str, label: &str, value: slint::SharedString, on: bool, dim: bool| MenuRow {
        id: id.into(),
        label: label.into(),
        value,
        on,
        dim,
    };
    let mut rows = Vec::new();
    let update = ui.get_update_version();
    if !update.is_empty() {
        rows.push(row("update", &format!("Install update v{update}"), "New".into(), true, false));
    }
    rows.push(row("boost", "Boost while playing", on_off(cfg.enabled), cfg.enabled, false));
    let off = !cfg.enabled;
    rows.push(row("freeze", "Freeze background apps", on_off(cfg.freeze_apps), cfg.freeze_apps, off));
    rows.push(row(
        "apps",
        "Apps to freeze",
        format!("{} selected", cfg.freeze_list.len()).into(),
        false,
        off || !cfg.freeze_apps,
    ));
    rows.push(row("power", "Best performance power mode", on_off(cfg.power_mode), cfg.power_mode, off));
    rows.push(row("priority", "High priority for games", on_off(cfg.game_priority), cfg.game_priority, off));
    if active {
        rows.push(row("end", "End boost now", "Active".into(), true, false));
    }
    rows.push(row("rescan", "Rescan library", "".into(), false, false));
    rows.push(row("exit", "Exit XBGG", "".into(), false, false));

    let count = rows.len() as i32;
    ui.set_menu_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_menu_index(ui.get_menu_index().clamp(0, count - 1));
}

fn refresh_apps(app: &App, ui: &AppWindow) {
    let rows: Vec<AppRow> = app
        .app_rows
        .borrow()
        .iter()
        .map(|a| AppRow {
            name: a.name.trim_end_matches(".exe").into(),
            detail: if a.running() {
                format!(
                    "Running · {} process{} · {}",
                    a.processes,
                    if a.processes == 1 { "" } else { "es" },
                    boost::mib(a.memory)
                )
                .into()
            } else {
                "Not running".into()
            },
            on: a.selected,
        })
        .collect();
    let count = rows.len() as i32;
    ui.set_app_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_app_index(ui.get_app_index().clamp(0, (count - 1).max(0)));
}

fn start_boost(
    app: &App,
    ui: &AppWindow,
    cfg: xbgg_core::boost::BoostConfig,
    game_dir: Option<std::path::PathBuf>,
    game_pid: Option<u32>,
) {
    let weak = ui.as_weak();
    app.boost.borrow_mut().start(cfg, game_dir, game_pid, move |event| {
        let _ = weak.upgrade_in_event_loop(move |ui| on_boost_event(&ui, event));
    });
    ui.set_boost_chip("starting…".into());
}

fn on_boost_event(ui: &AppWindow, event: boost::Event) {
    match event {
        boost::Event::Started(r) => {
            let gained = r.ram_after.saturating_sub(r.ram_before);
            let mut chip = Vec::new();
            if !r.frozen_apps.is_empty() {
                chip.push(format!("{} apps frozen", r.frozen_apps.len()));
            }
            if gained >= 100 << 20 {
                chip.push(format!("+{} RAM", boost::gib(gained)));
            }
            if r.power_changed {
                chip.push("best performance".into());
            }
            if chip.is_empty() {
                chip.push("on".into());
            }
            ui.set_boost_chip(chip.join(" · ").into());

            let mut status = String::from("Boost on");
            if !r.frozen_apps.is_empty() {
                let names: Vec<_> = r.frozen_apps.iter().take(3).map(|n| n.trim_end_matches(".exe")).collect();
                let more = r.frozen_apps.len().saturating_sub(3);
                status += &format!(" — froze {}{}", names.join(", "), if more > 0 { format!(" +{more}") } else { String::new() });
                status += &format!(" · {} released", boost::mib(r.trimmed_bytes));
            }
            if !r.skipped.is_empty() {
                status += &format!(" · {} not allowed", r.skipped.len());
            }
            ui.set_status(status.into());
        }
        boost::Event::GameFound => {}
        boost::Event::Ended(s) => {
            ui.set_boost_chip("".into());
            let mut status = format!("Boost ended — {} after {}", s.reason, boost::duration(s.duration));
            if s.restore.resumed > 0 {
                status += &format!(" · {} processes resumed", s.restore.resumed);
            }
            if s.restore.power_restored {
                status += " · power mode restored";
            }
            ui.set_status(status.into());
            if ui.get_panel() == 1 {
                ui.invoke_menu_open();
            }
        }
    }
}

fn wire_callbacks(app: &Rc<App>, ui: &AppWindow) {
    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_launch(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if app.last_launch.get().is_some_and(|t| t.elapsed() < LAUNCH_COOLDOWN) {
                return;
            }
            platform::allow_foreground_handoff();
            let result = app
                .library
                .borrow_mut()
                .launch(index as usize)
                .map(|(g, pid)| (g.title.clone(), g.id.clone(), g.install_dir.clone(), pid));
            match result {
                Ok((title, id, install_dir, pid)) => {
                    app.last_launch.set(Some(Instant::now()));
                    app.set_status(&ui, format!("Launching {title}…"));
                    let cfg = app.library.borrow().boost().clone();
                    if cfg.enabled {
                        start_boost(&app, &ui, cfg, install_dir, pid);
                    }
                    // Move the game to the front once the launch has settled,
                    // so the carousel does not jump while the user is watching.
                    let (app, weak) = (app.clone(), ui.as_weak());
                    Timer::single_shot(LAUNCH_COOLDOWN, move || {
                        if let Some(ui) = weak.upgrade() {
                            app.library.borrow_mut().resort();
                            app.sync(&ui, Some(&id));
                        }
                    });
                }
                Err(e) => app.set_status(&ui, format!("Couldn't launch: {e:#}")),
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_add_program(move || {
            let Some(ui) = weak.upgrade() else { return };
            if app.picking_file.replace(true) {
                return;
            }
            ui.set_input_blocked(true);
            let picker = rfd::AsyncFileDialog::new()
                .set_title("Add a game or app to XBGG")
                .add_filter("Games, apps and shortcuts", ADDABLE_EXTENSIONS)
                .set_parent(&ui.window().window_handle())
                .pick_file();

            let (task_app, weak) = (app.clone(), weak.clone());
            let spawned = slint::spawn_local(async move {
                let app = task_app;
                let picked = picker.await;
                app.picking_file.set(false);
                let Some(ui) = weak.upgrade() else { return };
                ui.set_input_blocked(false);
                let Some(file) = picked else { return };
                let path = file.path().to_path_buf();
                // Keep the mutable borrow in its own statement: reading the
                // library while `borrow_mut()` is alive would panic.
                let result = app.library.borrow_mut().add_custom(&path);
                let added = result.map(|i| {
                    let lib = app.library.borrow();
                    let g = &lib.games()[i];
                    (g.title.clone(), g.id.clone())
                });
                match added {
                    Ok((title, id)) => {
                        app.sync(&ui, Some(&id));
                        app.set_status(&ui, format!("Added {title}"));
                    }
                    Err(e) => app.set_status(&ui, format!("Couldn't add: {e:#}")),
                }
            });
            if spawned.is_err() {
                app.picking_file.set(false);
                ui.set_input_blocked(false);
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_remove_entry(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let info = app
                .library
                .borrow()
                .games()
                .get(index as usize)
                .map(|g| (g.title.clone(), g.source));
            let Some((title, source)) = info else { return };
            let result = app.library.borrow_mut().remove(index as usize);
            match result {
                Ok(()) => {
                    app.sync(&ui, None);
                    let verb = if source == Source::Custom { "Removed" } else { "Hid" };
                    app.set_status(&ui, format!("{verb} {title}"));
                }
                Err(e) => app.set_status(&ui, format!("Couldn't remove: {e:#}")),
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_rescan(move || {
            let Some(ui) = weak.upgrade() else { return };
            let keep = app
                .library
                .borrow()
                .games()
                .get(ui.get_selected() as usize)
                .map(|g| g.id.clone());
            app.library.borrow_mut().rescan();
            app.sync(&ui, keep.as_deref());
            if !report_warnings(&app, &ui) {
                let n = app.library.borrow().games().len();
                app.set_status(&ui, format!("Library refreshed: {n} entries"));
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_layout_changed(move || {
            // Debounced: resizing fires this many times in a row.
            let (app2, weak) = (app.clone(), weak.clone());
            app.layout_timer.start(TimerMode::SingleShot, Duration::from_millis(150), move || {
                if let Some(ui) = weak.upgrade() {
                    app2.sync(&ui, None);
                }
            });
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_focused(move |index| {
            if let Some(ui) = weak.upgrade() {
                app.load_hero(&ui, index);
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_install_update(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(release) = app.pending_update.lock().ok().and_then(|g| g.clone()) else { return };
            ui.set_input_blocked(true);
            ui.set_status(format!("Downloading v{}…", release.version).into());

            let weak = ui.as_weak();
            std::thread::spawn(move || {
                let result = update::install(&release);
                let _ = weak.upgrade_in_event_loop(move |ui| match result {
                    Ok(exe) => {
                        ui.set_status("Update installed, restarting…".into());
                        let args: Vec<String> = std::env::args().skip(1).collect();
                        match std::process::Command::new(&exe)
                            .args(args)
                            .env_remove("XBGG_PRETEND_VERSION")
                            .env(RESTART_ENV, "1")
                            .spawn()
                        {
                            Ok(_) => {
                                let _ = slint::quit_event_loop();
                            }
                            Err(e) => {
                                ui.set_input_blocked(false);
                                ui.set_status(format!("Updated; restart XBGG to finish ({e})").into());
                            }
                        }
                    }
                    Err(e) => {
                        ui.set_input_blocked(false);
                        ui.set_status(format!("Update failed: {e:#}").into());
                    }
                });
            });
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_menu_open(move || {
            if let Some(ui) = weak.upgrade() {
                refresh_menu(&app, &ui);
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_menu_select(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let toggle = |f: fn(&mut xbgg_core::boost::BoostConfig)| {
                if let Err(e) = app.library.borrow_mut().update_boost(f) {
                    app.set_status(&ui, format!("Couldn't save settings: {e:#}"));
                }
            };
            match id.as_str() {
                "boost" => toggle(|c| c.enabled = !c.enabled),
                "freeze" => toggle(|c| c.freeze_apps = !c.freeze_apps),
                "power" => toggle(|c| c.power_mode = !c.power_mode),
                "priority" => toggle(|c| c.game_priority = !c.game_priority),
                "apps" => ui.invoke_open_apps(),
                "end" => {
                    app.boost.borrow_mut().stop();
                    app.set_status(&ui, "Ending boost…");
                }
                "update" => {
                    ui.set_panel(0);
                    ui.set_dialog(3);
                }
                "rescan" => {
                    ui.set_panel(0);
                    ui.invoke_rescan();
                }
                "exit" => {
                    ui.set_panel(0);
                    ui.set_dialog(1);
                }
                _ => {}
            }
            if ui.get_panel() == 1 {
                refresh_menu(&app, &ui);
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_apps_open(move || {
            if let Some(ui) = weak.upgrade() {
                let apps = xbgg_core::boost::apps::candidates(app.library.borrow().boost());
                *app.app_rows.borrow_mut() = apps;
                refresh_apps(&app, &ui);
            }
        });
    }

    {
        let (app, weak) = (app.clone(), ui.as_weak());
        ui.on_app_toggle(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let Some(name) = app.app_rows.borrow().get(index as usize).map(|a| a.name.clone()) else { return };
            let result = app.library.borrow_mut().update_boost(|c| c.toggle_app(&name));
            match result {
                Ok(()) => {
                    let selected = app.library.borrow().boost().freeze_list.contains(&name);
                    if let Some(row) = app.app_rows.borrow_mut().get_mut(index as usize) {
                        row.selected = selected;
                    }
                    refresh_apps(&app, &ui);
                }
                Err(e) => app.set_status(&ui, format!("Couldn't save settings: {e:#}")),
            }
        });
    }

    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
}

/// Shows the first new warning in the status line (all go to stderr).
/// Returns whether there was one.
fn report_warnings(app: &App, ui: &AppWindow) -> bool {
    let warnings = app.library.borrow_mut().take_warnings();
    for w in &warnings {
        eprintln!("xbgg: {w}");
    }
    match warnings.into_iter().next() {
        Some(first) => {
            app.set_status(ui, first);
            true
        }
        None => false,
    }
}

fn start_clock(ui: &AppWindow) -> Timer {
    let tick = {
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                let now = chrono::Local::now().format("%H:%M").to_string();
                // Only touch the property when the text changes, to avoid redraws.
                if ui.get_clock() != now.as_str() {
                    ui.set_clock(now.into());
                }
            }
        }
    };
    tick();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), tick);
    timer
}

fn start_gamepad(ui: &AppWindow) {
    let weak = ui.as_weak();
    gamepad::spawn(platform::process_is_foreground, move |action| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            match action {
                PadAction::Left => ui.invoke_nav(-1),
                PadAction::Right => ui.invoke_nav(1),
                PadAction::PageLeft => ui.invoke_nav(-5),
                PadAction::PageRight => ui.invoke_nav(5),
                PadAction::Activate => ui.invoke_activate(),
                PadAction::Back => ui.invoke_back(),
                PadAction::Add => ui.invoke_add(),
                PadAction::Secondary => ui.invoke_secondary(),
                PadAction::Refresh => ui.invoke_refresh(),
                PadAction::Menu => ui.invoke_menu(),
                PadAction::Up => ui.invoke_nav_vertical(-1),
                PadAction::Down => ui.invoke_nav_vertical(1),
            }
        });
    });
}

fn start_update_check(app: &App, ui: &AppWindow) {
    let pending = app.pending_update.clone();
    let weak = ui.as_weak();
    std::thread::spawn(move || match update::check() {
        Ok(Some(release)) => {
            let version = release.version.to_string();
            if let Ok(mut slot) = pending.lock() {
                *slot = Some(release);
            }
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_update_version(version.into()));
        }
        Ok(None) => {}
        // Offline or rate-limited: not worth bothering the player about.
        Err(e) => eprintln!("xbgg: update check failed: {e:#}"),
    });
}

fn to_tile(g: &Game, cover: Option<Image>) -> GameTile {
    GameTile {
        id: g.id.as_str().into(),
        title: g.title.as_str().into(),
        source: g.source.label().into(),
        has_cover: cover.is_some(),
        cover: cover.unwrap_or_default(),
        accent: accent_for(&g.title),
        initials: initials(&g.title).into(),
    }
}

/// A stable, muted color derived from the title, used when no artwork exists.
fn accent_for(title: &str) -> Color {
    let hash = title.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let hue = (hash % 360) as f32;
    let (s, l) = (0.45f32, 0.34f32);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match hue as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round() as u8;
    Color::from_rgb_u8(to(r), to(g), to(b))
}

fn initials(title: &str) -> String {
    let letters: String = title
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if letters.is_empty() { "?".into() } else { letters }
}

/// `xbgg --list`: prints what the scanners found (for debugging).
fn list_library() -> anyhow::Result<()> {
    let lib = Library::load();
    for w in &lib.warnings {
        eprintln!("warning: {w}");
    }
    for g in lib.games() {
        println!(
            "{:<11} {:<48} cover:{} hero:{}  {}",
            g.source.label(),
            g.title,
            if g.cover.is_some() { "yes" } else { "no " },
            if g.hero.is_some() { "yes" } else { "no " },
            g.id
        );
    }
    println!("{} entries", lib.games().len());
    Ok(())
}
