#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod art;
mod gamepad;
mod platform;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{Color, ComponentHandle, Image, ModelRc, Timer, TimerMode, VecModel};
use xbgg_core::{Game, Library, Source, ADDABLE_EXTENSIONS};

use gamepad::PadAction;

slint::include_modules!();

/// Ignore repeated "Play" presses while a launch is already in progress.
const LAUNCH_COOLDOWN: Duration = Duration::from_secs(3);
const STATUS_DURATION: Duration = Duration::from_secs(5);

struct App {
    library: RefCell<Library>,
    model: Rc<VecModel<GameTile>>,
    status_timer: Timer,
    last_launch: Cell<Option<Instant>>,
    picking_file: Cell<bool>,
    hero: art::HeroLoader,
}

impl App {
    fn load_hero(&self, ui: &AppWindow, index: i32) {
        let path = usize::try_from(index)
            .ok()
            .and_then(|i| self.library.borrow().games().get(i).and_then(|g| g.hero.clone()));
        self.hero.request(ui, path);
    }

    /// Pushes the library into the UI, optionally focusing the entry with `select_id`.
    fn sync(&self, ui: &AppWindow, select_id: Option<&str>) {
        let lib = self.library.borrow();
        self.model.set_vec(lib.games().iter().map(to_tile).collect::<Vec<_>>());
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

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        return list_library();
    }
    let windowed = args.iter().any(|a| a == "--windowed");

    let ui = AppWindow::new()?;
    let model = Rc::new(VecModel::<GameTile>::default());
    ui.set_games(ModelRc::from(model.clone()));

    let app = Rc::new(App {
        library: RefCell::new(Library::load()),
        model,
        status_timer: Timer::default(),
        last_launch: Cell::new(None),
        picking_file: Cell::new(false),
        hero: art::HeroLoader::default(),
    });
    app.sync(&ui, None);
    report_warnings(&app, &ui);

    wire_callbacks(&app, &ui);
    let _clock = start_clock(&ui);
    start_gamepad(&ui);

    if !windowed {
        ui.window().set_fullscreen(true);
    }
    ui.run()?;
    Ok(())
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
                .map(|g| (g.title.clone(), g.id.clone()));
            match result {
                Ok((title, id)) => {
                    app.last_launch.set(Some(Instant::now()));
                    app.set_status(&ui, format!("Launching {title}…"));
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
                let (Some(ui), Some(file)) = (weak.upgrade(), picked) else { return };
                let path = file.path().to_path_buf();
                let added = app.library.borrow_mut().add_custom(&path).map(|i| {
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
            let n = app.library.borrow().games().len();
            app.set_status(&ui, format!("Library refreshed: {n} entries"));
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

    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
}

fn report_warnings(app: &App, ui: &AppWindow) {
    let mut lib = app.library.borrow_mut();
    for w in &lib.warnings {
        eprintln!("xbgg: {w}");
    }
    if let Some(first) = lib.warnings.first() {
        let text = first.clone();
        drop(lib);
        app.set_status(ui, text);
        app.library.borrow_mut().warnings.clear();
    } else {
        lib.warnings.clear();
    }
}

fn start_clock(ui: &AppWindow) -> Timer {
    let tick = {
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_clock(chrono::Local::now().format("%H:%M").to_string().into());
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
    gamepad::spawn(move |action| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !platform::is_foreground(&ui) {
                return;
            }
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
            }
        });
    });
}

fn to_tile(g: &Game) -> GameTile {
    let cover = g.cover.as_deref().and_then(|p| Image::load_from_path(p).ok());
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
