mod config;
mod tasks;
use config::Config;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;
use std::{
    cell::RefCell,
    ffi::OsStr,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    rc::Rc,
    time::Duration,
};

#[derive(Clone, Deserialize)]
struct Calendar {
    id: String,
    name: String,
}
#[derive(Deserialize)]
struct Event {
    title: String,
    time: String,
    calendar: String,
    description: String,
    location: String,
}
#[derive(Deserialize)]
struct Day {
    date: String,
    label: String,
    events: Vec<Event>,
}
#[derive(Deserialize)]
struct Snapshot {
    calendars: Vec<Calendar>,
    days: Vec<Day>,
    start: String,
    timezone: String,
}
type State = Rc<RefCell<Ui>>;
struct Ui {
    window: gtk::ApplicationWindow,
    chooser: Option<gtk::FileChooserDialog>,
    tasks_box: gtk::Box,
    tasks_button: gtk::Button,
    document: Option<tasks::Document>,
    task_filter: String,
    show_completed: bool,
    panel: gtk::Box,
    config: Config,
    stack: gtk::Stack,
    agenda: gtk::Box,
    settings: gtk::Box,
    month: gtk::Calendar,
    status: gtk::Label,
    calendars: Vec<Calendar>,
    start: Option<String>,
    generation: u64,
    process: Option<gio::Subprocess>,
    system_theme: Option<glib::GString>,
    system_dark: bool,
}
fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(54);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}
fn column(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
fn row(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}
fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}
fn icon(name: &str, fallback: &str) -> gtk::Button {
    let b = if gdk::Display::default()
        .is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name))
    {
        gtk::Button::from_icon_name(name)
    } else {
        gtk::Button::with_label(fallback)
    };
    b.add_css_class("flat");
    b.set_tooltip_text(Some(fallback));
    b
}
fn ipc(command: &str) -> Option<serde_json::Value> {
    let path = std::env::var_os("MANGO_INSTANCE_SIGNATURE")?;
    let mut socket = UnixStream::connect(path).ok()?;
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    socket
        .set_write_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    writeln!(socket, "{command}").ok()?;
    let mut line = String::new();
    BufReader::new(socket).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}
fn theme(ui: &Ui) {
    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_theme_name(if ui.config.theme == "system" {
            ui.system_theme.as_deref()
        } else {
            Some("Adwaita")
        });
        s.set_gtk_application_prefer_dark_theme(match ui.config.theme.as_str() {
            "dark" => true,
            "light" => false,
            _ => ui.system_dark,
        });
    }
}
fn persist(state: &State) {
    if let Err(e) = state.borrow().config.save() {
        state
            .borrow()
            .status
            .set_text(&format!("Could not save settings: {e}"));
    }
}
fn load(state: &State) {
    let mut ui = state.borrow_mut();
    ui.generation += 1;
    let generation = ui.generation;
    if let Some(old) = ui.process.take() {
        old.force_exit();
    }
    clear(&ui.agenda);
    ui.agenda.append(&label("Loading your agenda…", "dim"));
    let mut request = serde_json::to_value(&ui.config).unwrap();
    request["start"] = serde_json::to_value(&ui.start).unwrap();
    let json = request.to_string();
    let args: Vec<&OsStr> = vec![
        OsStr::new(&ui.config.python),
        OsStr::new("-c"),
        OsStr::new(include_str!("../data/backend.py")),
        OsStr::new(&json),
    ];
    let process = match gio::Subprocess::newv(
        &args,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
    ) {
        Ok(p) => p,
        Err(e) => {
            clear(&ui.agenda);
            ui.agenda
                .append(&label(&format!("Could not start Python: {e}"), "error"));
            return;
        }
    };
    ui.process = Some(process.clone());
    let weak = Rc::downgrade(state);
    drop(ui);
    let timed_out = Rc::new(std::cell::Cell::new(false));
    let flag = timed_out.clone();
    let p = process.clone();
    let timeout = glib::timeout_add_local_once(Duration::from_secs(20), move || {
        flag.set(true);
        p.force_exit();
    });
    glib::MainContext::default().spawn_local(async move {
        let result = process.communicate_utf8_future(None).await;
        if !timed_out.get() { timeout.remove(); }
        let Some(state) = weak.upgrade() else { return; };
        let mut ui = state.borrow_mut();
        if ui.generation != generation { return; }
        ui.process = None;
        let had_calendars = !ui.calendars.is_empty();
        clear(&ui.agenda);
        match result {
            Ok((stdout, stderr)) if process.is_successful() => {
                match serde_json::from_str::<Snapshot>(stdout.as_deref().unwrap_or("")) {
                    Ok(snapshot) => render(&mut ui, snapshot),
                    Err(e) => { ui.agenda.append(&label(&format!("Could not read the calendar response: {e}"), "error")); }
                }
                let _ = stderr;
            }
            Ok((_, stderr)) => { ui.agenda.append(&label(if timed_out.get() { "Reading calendars took too long. Try Refresh or check khal list in a terminal." } else { stderr.as_deref().unwrap_or("Could not read calendars. Check khal list in a terminal.") }, "error")); }
            Err(e) => { ui.agenda.append(&label(&format!("Could not read calendars: {e}"), "error")); }
        }
        let initialize_settings = !had_calendars && !ui.calendars.is_empty()
            && ui.stack.visible_child_name().as_deref() == Some("settings");
        drop(ui);
        if initialize_settings { settings(&state); }
    });
}
fn render(ui: &mut Ui, snapshot: Snapshot) {
    ui.calendars = snapshot.calendars;
    let count: usize = snapshot.days.iter().map(|d| d.events.len()).sum();
    let days = ui.config.days_ahead + 1;
    if ui.stack.visible_child_name().as_deref() != Some("tasks") {
        ui.status.set_text(&format!(
            "{} · {} {} · {}",
            snapshot.start,
            days,
            if days == 1 { "day" } else { "days" },
            snapshot.timezone
        ));
    }
    if count == 0 {
        ui.agenda.append(&label(
            if ui
                .calendars
                .iter()
                .all(|c| ui.config.excluded.contains(&c.id))
            {
                "No calendars selected. Choose calendars in Settings."
            } else {
                "No events in this date range."
            },
            "dim",
        ));
        return;
    }
    for day in snapshot.days {
        if day.events.is_empty() {
            continue;
        }
        let heading = label(&day.label, "day");
        heading.set_tooltip_text(Some(&day.date));
        ui.agenda.append(&heading);
        for event in day.events {
            let expander = gtk::Expander::new(None);
            expander.add_css_class("event");
            let heading = column(3);
            heading.append(&label(&event.time, "time"));
            heading.append(&label(&event.title, "event-title"));
            heading.append(&label(&event.calendar, "dim"));
            expander.set_label_widget(Some(&heading));
            let details = column(6);
            details.add_css_class("details");
            if !event.location.is_empty() {
                details.append(&label(&event.location, "subtitle"));
            }
            details.append(&label(
                if event.description.is_empty() {
                    "No additional details."
                } else {
                    &event.description
                },
                "dim",
            ));
            expander.set_child(Some(&details));
            ui.agenda.append(&expander);
        }
    }
}
fn todo_path(text: &str) -> std::path::PathBuf {
    if let Some(rest) = text.strip_prefix("~/") {
        std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else {
        text.into()
    }
}
fn show_tasks(state: &State) {
    let path = todo_path(&state.borrow().config.todo_file);
    let result = tasks::Document::load(&path);
    let mut ui = state.borrow_mut();
    ui.stack.set_visible_child_name("tasks");
    ui.tasks_button.set_visible(true);
    match result {
        Ok(doc) => {
            ui.document = Some(doc);
            ui.status.set_text("Tasks · todo.txt");
        }
        Err(e) => {
            ui.document = None;
            ui.status.set_text(&e.to_string());
        }
    }
    drop(ui);
    render_tasks(state);
}
fn render_tasks(state: &State) {
    let ui = state.borrow();
    clear(&ui.tasks_box);
    let filter = gtk::Entry::builder()
        .placeholder_text("Filter: text, (A), @context, +project")
        .text(&ui.task_filter)
        .build();
    ui.tasks_box.append(&filter);
    let st = state.clone();
    filter.connect_activate(move |entry| {
        st.borrow_mut().task_filter = entry.text().to_string();
        render_tasks(&st);
    });
    let completed_row = row(12);
    let text = label("Show completed tasks", "subtitle");
    text.set_hexpand(true);
    completed_row.append(&text);
    let completed = gtk::Switch::builder().active(ui.show_completed).build();
    completed_row.append(&completed);
    ui.tasks_box.append(&completed_row);
    let st = state.clone();
    completed.connect_active_notify(move |b| {
        st.borrow_mut().show_completed = b.is_active();
        render_tasks(&st);
    });
    let add_row = row(6);
    let entry = gtk::Entry::builder()
        .placeholder_text("New task, including priority or tags")
        .hexpand(true)
        .build();
    let add = gtk::Button::with_label("Add");
    add_row.append(&entry);
    add_row.append(&add);
    ui.tasks_box.append(&add_row);
    let st = state.clone();
    let input = entry.clone();
    add.connect_clicked(move |_| task_save(&st, None, input.text().as_str()));
    let st = state.clone();
    entry.connect_activate(move |e| task_save(&st, None, e.text().as_str()));
    let Some(doc) = &ui.document else {
        ui.tasks_box.append(&label(
            "Select an existing todo.txt file in Settings.",
            "dim",
        ));
        return;
    };
    let mut count = 0;
    for (i, raw) in doc.lines.iter().enumerate() {
        let text = raw.trim_end_matches(['\r', '\n']);
        let done = text.starts_with("x ");
        if text.trim().is_empty()
            || (done && !ui.show_completed)
            || !text.to_lowercase().contains(&ui.task_filter.to_lowercase())
        {
            continue;
        }
        count += 1;
        let r = column(6);
        r.add_css_class("event");
        let input = gtk::Entry::builder().text(text).hexpand(true).build();
        r.append(&input);
        let controls = row(6);
        let save = gtk::Button::with_label("Save task");
        let complete = gtk::Button::with_label(if done { "Reopen task" } else { "Complete task" });
        controls.append(&save);
        controls.append(&complete);
        r.append(&controls);
        ui.tasks_box.append(&r);
        let st = state.clone();
        let e = input.clone();
        save.connect_clicked(move |_| task_save(&st, Some(i), e.text().as_str()));
        let st = state.clone();
        complete.connect_clicked(move |_| {
            let value = input.text();
            let updated = if done {
                let rest = value.strip_prefix("x ").unwrap_or(&value);
                if rest.len() >= 11
                    && rest.as_bytes()[..10]
                        .iter()
                        .all(|b| b.is_ascii_digit() || *b == b'-')
                    && rest.as_bytes()[4] == b'-'
                    && rest.as_bytes()[7] == b'-'
                    && rest.as_bytes()[10] == b' '
                {
                    rest[11..].to_owned()
                } else {
                    rest.to_owned()
                }
            } else {
                let date = glib::DateTime::now_local()
                    .and_then(|d| d.format("%Y-%m-%d"))
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                format!("x {date} {value}")
            };
            task_save(&st, Some(i), &updated);
        });
    }
    if count == 0 {
        ui.tasks_box.append(&label("No matching tasks.", "dim"));
    }
}
fn task_save(state: &State, index: Option<usize>, text: &str) {
    let result = state
        .borrow_mut()
        .document
        .as_mut()
        .map(|doc| doc.save(index, text));
    match result {
        Some(Ok(())) => {
            state.borrow().status.set_text("Task saved");
            render_tasks(state);
        }
        Some(Err(e)) => state.borrow().status.set_text(&e.to_string()),
        None => state
            .borrow()
            .status
            .set_text("Choose a readable todo.txt file first."),
    }
}
fn settings(state: &State) {
    let ui = state.borrow();
    clear(&ui.settings);
    ui.settings.append(&label("Make it yours.", "title"));
    let r = row(12);
    let text = label("Top margin (px)", "subtitle");
    text.set_hexpand(true);
    r.append(&text);
    let margin = gtk::SpinButton::with_range(0., 500., 1.);
    margin.set_value(ui.config.top_margin.into());
    margin.set_tooltip_text(Some(
        "Distance from the top of the screen in logical pixels",
    ));
    r.append(&margin);
    ui.settings.append(&r);
    let st = state.clone();
    margin.connect_value_changed(move |spin| {
        {
            let mut ui = st.borrow_mut();
            ui.config.top_margin = spin.value_as_int() as u32;
            ui.panel.set_margin_top(spin.value_as_int());
        }
        persist(&st);
    });
    let r = row(12);
    let text = label("Days ahead", "subtitle");
    text.set_hexpand(true);
    r.append(&text);
    let spin = gtk::SpinButton::with_range(0., 90., 1.);
    spin.set_value(ui.config.days_ahead.into());
    spin.set_tooltip_text(Some(
        "Always includes the selected day, plus this many days.",
    ));
    r.append(&spin);
    ui.settings.append(&r);
    let st = state.clone();
    spin.connect_value_changed(move |v| {
        st.borrow_mut().config.days_ahead = v.value_as_int() as u32;
        persist(&st);
        load(&st);
    });
    let r = row(12);
    let text = label("Theme", "subtitle");
    text.set_hexpand(true);
    r.append(&text);
    let dropdown = gtk::DropDown::from_strings(&["GTK theme", "Light", "Dark"]);
    dropdown.set_selected(match ui.config.theme.as_str() {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    });
    r.append(&dropdown);
    ui.settings.append(&r);
    let st = state.clone();
    dropdown.connect_selected_notify(move |d| {
        {
            let mut u = st.borrow_mut();
            u.config.theme = ["system", "light", "dark"][d.selected() as usize].into();
            theme(&u);
        }
        persist(&st);
    });
    let r = row(12);
    let text = label("Show month calendar", "subtitle");
    text.set_hexpand(true);
    r.append(&text);
    let switch = gtk::Switch::builder().active(ui.config.month_view).build();
    r.append(&switch);
    ui.settings.append(&r);
    let st = state.clone();
    switch.connect_active_notify(move |v| {
        let mut u = st.borrow_mut();
        u.config.month_view = v.is_active();
        u.month.set_visible(v.is_active());
        drop(u);
        persist(&st);
    });
    let r = row(12);
    let text = label("Enable tasks", "subtitle");
    text.set_hexpand(true);
    r.append(&text);
    let toggle = gtk::Switch::builder()
        .active(ui.config.tasks_enabled)
        .build();
    r.append(&toggle);
    ui.settings.append(&r);
    let st = state.clone();
    toggle.connect_active_notify(move |toggle| {
        let mut u = st.borrow_mut();
        u.config.tasks_enabled = toggle.is_active();
        u.tasks_button.set_visible(toggle.is_active());
        drop(u);
        persist(&st);
    });
    ui.settings.append(&label("todo.txt file", "subtitle"));
    let path_row = row(6);
    let path = gtk::Entry::builder()
        .text(&ui.config.todo_file)
        .hexpand(true)
        .build();
    let apply = gtk::Button::with_label("Apply path");
    path_row.append(&path);
    path_row.append(&apply);
    let browse = gtk::Button::with_label("Browse");
    path_row.append(&browse);
    ui.settings.append(&path_row);
    let st = state.clone();
    let st_browse = state.clone();
    let path_input = path.clone();
    browse.connect_clicked(move |_| {
        if st_browse.borrow().chooser.is_some() {
            return;
        }
        // Layer surfaces cannot be exported as an xdg-toplevel dialog parent.
        let chooser = gtk::FileChooserDialog::new(
            Some("Choose todo.txt"),
            None::<&gtk::Window>,
            gtk::FileChooserAction::Open,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                ("Choose", gtk::ResponseType::Accept),
            ],
        );
        let path = todo_path(&st_browse.borrow().config.todo_file);
        let weak = Rc::downgrade(&st_browse);
        let input = path_input.clone();
        chooser.connect_response(move |chooser, response| {
            let Some(st) = weak.upgrade() else {
                chooser.destroy();
                return;
            };
            if response == gtk::ResponseType::Accept
                && let Some(path) = chooser.file().and_then(|f| f.path())
            {
                let text = path.to_string_lossy().to_string();
                input.set_text(&text);
                st.borrow_mut().config.todo_file = text;
                persist(&st);
            }
            chooser.destroy();
            let mut ui = st.borrow_mut();
            ui.chooser = None;
            if gtk4_layer_shell::is_supported() {
                ui.window.set_keyboard_mode(KeyboardMode::Exclusive);
            }
            ui.window.present();
        });
        let mut ui = st_browse.borrow_mut();
        if gtk4_layer_shell::is_supported() {
            ui.window.set_keyboard_mode(KeyboardMode::None);
        }
        ui.window.hide();
        ui.chooser = Some(chooser.clone());
        drop(ui);
        chooser.present();
        if let Some(parent) = path.parent() {
            let _ = chooser.set_current_folder(Some(&gio::File::for_path(parent)));
        }
    });
    apply.connect_clicked(move |_| {
        st.borrow_mut().config.todo_file = path.text().to_string();
        persist(&st);
    });
    ui.settings.append(&label("Calendars", "day"));
    if ui.calendars.is_empty() {
        ui.settings.append(&label(
            "Calendars will appear after a successful refresh.",
            "dim",
        ));
    }
    for calendar in &ui.calendars {
        let r = row(12);
        let text = label(&calendar.name, "subtitle");
        text.set_hexpand(true);
        r.append(&text);
        let switch = gtk::Switch::builder()
            .active(!ui.config.excluded.contains(&calendar.id))
            .build();
        switch.set_tooltip_text(Some(&calendar.id));
        r.append(&switch);
        ui.settings.append(&r);
        let id = calendar.id.clone();
        let st = state.clone();
        switch.connect_active_notify(move |v| {
            {
                let mut u = st.borrow_mut();
                u.config.excluded.retain(|x| x != &id);
                if !v.is_active() {
                    u.config.excluded.push(id.clone());
                }
            }
            persist(&st);
            load(&st);
        });
    }
    let done = gtk::Button::with_label("Done");
    ui.settings.append(&done);
    let stack = ui.stack.clone();
    done.connect_clicked(move |_| stack.set_visible_child_name("agenda"));
    ui.stack.set_visible_child_name("settings");
}
fn build(app: &gtk::Application, config: Config, focused: bool, open_tasks: bool) -> State {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Khal Agenda")
        .decorated(false)
        .build();
    window.add_css_class("agenda-overlay");
    let mut available_height = 900;
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_namespace(Some("khal-agenda"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        let name = (if focused { None } else { ipc("get cursorpos") })
            .and_then(|v| v["monitor"].as_str().map(str::to_owned))
            .or_else(|| {
                ipc("get all-monitors").and_then(|v| {
                    v["monitors"]
                        .as_array()?
                        .iter()
                        .find(|m| m["active"] == true)?["name"]
                        .as_str()
                        .map(str::to_owned)
                })
            });
        if let Some(display) = gdk::Display::default() {
            let monitors = display.monitors();
            for i in 0..monitors.n_items() {
                if let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>()
                    && name.is_some()
                    && monitor.connector().as_deref() == name.as_deref()
                {
                    available_height = monitor.geometry().height();
                    window.set_monitor(Some(&monitor));
                    break;
                }
            }
        }
    } else {
        window.set_default_size(490, 720);
    }
    let overlay = gtk::Overlay::new();
    let backdrop = gtk::Button::new();
    backdrop.add_css_class("backdrop");
    backdrop.set_can_focus(false);
    let a = app.clone();
    backdrop.connect_clicked(move |_| a.quit());
    overlay.set_child(Some(&backdrop));
    let panel = column(0);
    panel.add_css_class("panel");
    panel.set_halign(gtk::Align::End);
    panel.set_valign(gtk::Align::Start);
    panel.set_margin_top(config.top_margin as i32);
    panel.set_margin_end(16);
    panel.set_margin_bottom(16);
    panel.set_size_request(480, -1);
    let header = row(10);
    header.add_css_class("header");
    let titles = column(4);
    titles.set_hexpand(true);
    let heading = label("Your days ahead.", "title");
    titles.append(&heading);
    titles.append(&label("Khal Agenda", "dim"));
    header.append(&titles);
    let refresh = icon("view-refresh-symbolic", "Refresh");
    let preferences = icon("emblem-system-symbolic", "Settings");
    let close = icon("window-close-symbolic", "Close");
    header.append(&refresh);
    header.append(&preferences);
    header.append(&close);
    panel.append(&header);
    let stack = gtk::Stack::new();
    let page = column(8);
    let month = gtk::Calendar::new();
    month.add_css_class("month");
    month.set_visible(config.month_view);
    page.append(&month);
    let agenda = column(2);
    agenda.add_css_class("agenda");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(260)
        .max_content_height((available_height - 430).clamp(180, 580))
        .propagate_natural_height(true)
        .child(&agenda)
        .build();
    page.append(&scroll);
    stack.add_named(&page, Some("agenda"));
    let settings_box = column(16);
    settings_box.add_css_class("settings");
    let sc = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(350)
        .max_content_height((available_height - 220).clamp(260, 680))
        .propagate_natural_height(true)
        .child(&settings_box)
        .build();
    stack.add_named(&sc, Some("settings"));
    let tabs = row(8);
    tabs.add_css_class("header");
    let agenda_button = gtk::Button::with_label("Agenda");
    let tasks_button = gtk::Button::with_label("Tasks");
    tasks_button.set_visible(config.tasks_enabled || open_tasks);
    tabs.append(&agenda_button);
    tabs.append(&tasks_button);
    tabs.set_visible(config.tasks_enabled || open_tasks);
    panel.append(&tabs);
    let tasks_box = column(8);
    tasks_box.add_css_class("agenda");
    let tasks_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(300)
        .max_content_height(580)
        .propagate_natural_height(true)
        .child(&tasks_box)
        .build();
    stack.add_named(&tasks_scroll, Some("tasks"));
    panel.append(&stack);
    let footer = row(8);
    footer.add_css_class("footer");
    let status = label("", "dim");
    status.set_hexpand(true);
    footer.append(&status);
    let today = gtk::Button::with_label("Today");
    footer.append(&today);
    panel.append(&footer);
    overlay.add_overlay(&panel);
    window.set_child(Some(&overlay));
    let gtk_settings = gtk::Settings::default();
    let state = Rc::new(RefCell::new(Ui {
        window: window.clone(),
        chooser: None,
        tasks_box,
        tasks_button: tasks_button.clone(),
        document: None,
        task_filter: String::new(),
        show_completed: false,
        panel,
        config,
        stack,
        agenda,
        settings: settings_box,
        month: month.clone(),
        status,
        calendars: vec![],
        start: None,
        generation: 0,
        process: None,
        system_theme: gtk_settings.as_ref().and_then(|s| s.gtk_theme_name()),
        system_dark: gtk_settings.is_some_and(|s| s.is_gtk_application_prefer_dark_theme()),
    }));
    theme(&state.borrow());
    let st = state.clone();
    tasks_button.connect_clicked(move |_| show_tasks(&st));
    let st = state.clone();
    agenda_button.connect_clicked(move |_| {
        st.borrow().stack.set_visible_child_name("agenda");
    });
    let tab_bar = tabs.clone();
    state
        .borrow()
        .tasks_button
        .connect_visible_notify(move |b| {
            tab_bar.set_visible(b.is_visible());
        });
    let st = state.clone();
    preferences.connect_clicked(move |_| settings(&st));
    let st = state.clone();
    refresh.connect_clicked(move |_| {
        if st.borrow().stack.visible_child_name().as_deref() == Some("tasks") {
            show_tasks(&st);
        } else {
            load(&st);
        }
    });
    let st = state.clone();
    month.connect_day_selected(move |m| {
        st.borrow_mut().start = m.date().format("%Y-%m-%d").ok().map(String::from);
        load(&st);
    });
    let heading_label = heading.clone();
    state
        .borrow()
        .stack
        .connect_visible_child_name_notify(move |stack| {
            heading_label.set_text(if stack.visible_child_name().as_deref() == Some("tasks") {
                "Your tasks."
            } else {
                "Your days ahead."
            });
        });
    let today_button = today.clone();
    state
        .borrow()
        .stack
        .connect_visible_child_name_notify(move |stack| {
            today_button.set_visible(stack.visible_child_name().as_deref() == Some("agenda"));
        });
    let st = state.clone();
    today.connect_clicked(move |_| {
        if let Ok(now) = glib::DateTime::now_local() {
            let month = st.borrow().month.clone();
            month.select_day(&now);
        }
        st.borrow_mut().start = None;
        load(&st);
    });
    let a = app.clone();
    close.connect_clicked(move |_| a.quit());
    let keys = gtk::EventControllerKey::new();
    let a = app.clone();
    let st = state.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key == gdk::Key::Escape {
            a.quit();
            glib::Propagation::Stop
        } else if key == gdk::Key::comma && mods.contains(gdk::ModifierType::CONTROL_MASK) {
            settings(&st);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    let st = state.clone();
    app.connect_shutdown(move |_| {
        if let Some(p) = st.borrow_mut().process.take() {
            p.force_exit();
        }
    });
    window.present();
    load(&state);
    if open_tasks {
        show_tasks(&state);
    }
    state
}
fn main() -> glib::ExitCode {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // Before GTK initialization or worker threads, select the layer-shell backend.
        unsafe {
            std::env::set_var("GDK_BACKEND", "wayland");
        }
    }
    if std::env::args().any(|a| a == "--version") {
        println!("khal-agenda {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Could not read settings: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    let app = gtk::Application::builder()
        .application_id("io.github.mikkelrask.KhalAgenda")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option(
        "focused",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open on the focused monitor for keyboard shortcuts",
        None,
    );
    app.add_main_option(
        "tasks",
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open todo.txt tasks directly",
        None,
    );
    let current: Rc<RefCell<Option<State>>> = Rc::new(RefCell::new(None));
    let task_mode = Rc::new(std::cell::Cell::new(false));
    let mode = task_mode.clone();
    let live = current.clone();
    let focused = Rc::new(std::cell::Cell::new(false));
    let flag = focused.clone();
    app.connect_command_line(move |app, command| {
        mode.set(command.options_dict().contains("tasks"));
        flag.set(command.options_dict().contains("focused"));
        app.activate();
        if mode.get()
            && let Some(state) = live.borrow().as_ref()
        {
            show_tasks(state);
        }
        glib::ExitCode::SUCCESS
    });
    app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(include_str!("../data/style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    app.connect_activate(move |app| {
        if let Some(chooser) = current
            .borrow()
            .as_ref()
            .and_then(|state| state.borrow().chooser.clone())
        {
            chooser.present();
            return;
        }
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            *current.borrow_mut() =
                Some(build(app, config.clone(), focused.get(), task_mode.get()));
        }
    });
    app.run()
}
