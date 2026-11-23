mod help;
pub(crate) mod theme;
mod views;

use crate::api::{Client, Connections, Endpoint, LogEntry, Proxies, Providers, Rule, Target};
use crate::{service::{ProfileInfo, Service}, settings::Settings};
use anyhow::{Context, Result, anyhow};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{DefaultTerminal, widgets::{ListState, TableState}};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    num::NonZeroUsize,
    path::Path,
    sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    Overview,
    #[default]
    Proxies,
    Rules,
    Conns,
    Logs,
    Profiles,
}

impl Page {
    const COUNT: usize = 6;
    const ALL: [Self; Self::COUNT] = [Self::Overview, Self::Proxies, Self::Rules, Self::Conns, Self::Logs, Self::Profiles];
    const NAMES: [&str; Self::COUNT] = ["Overview", "Proxies", "Rules", "Conns", "Logs", "Profiles"];

    fn index(self) -> usize { self as usize }

    fn interval(self) -> Option<Duration> {
        match self {
            Self::Overview | Self::Conns => Some(Duration::from_secs(1)),
            Self::Logs => Some(Duration::from_millis(150)),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Catalog {
    groups: Vec<crate::api::Group>,
    nodes: BTreeMap<Target, Node>,
    groups_delay: BTreeMap<String, u64>,
}

struct Node {
    kind: String,
    delay: Option<u64>,
}

impl Catalog {
    fn load(client: &Client) -> Result<Self> {
        let Proxies { proxies } = client.proxies()?;
        let providers = client.providers().ok();
        let mut catalog = Self::default();
        for (name, proxy) in proxies {
            let delay = proxy.history.last().map(|sample| sample.delay);
            if let Some(all) = proxy.all {
                if let Some(delay) = delay {
                    catalog.groups_delay.insert(name.clone(), delay);
                }
                catalog.groups.push(crate::api::Group { name, kind: proxy.kind, now: proxy.now, all });
            } else {
                let provider = (!proxy.provider.is_empty()).then_some(proxy.provider);
                catalog.nodes.insert(Target { name, provider }, Node { kind: proxy.kind, delay });
            }
        }
        if let Some(Providers { providers }) = providers {
            for (provider, entry) in providers {
                for proxy in entry.proxies {
                    let key = Target { name: proxy.name, provider: Some(provider.clone()) };
                    let delay = proxy.history.last().map(|sample| sample.delay);
                    catalog.nodes.entry(key).and_modify(|node| {
                        if node.delay.is_none() { node.delay = delay; }
                    }).or_insert(Node { kind: proxy.kind, delay });
                }
            }
        }
        Ok(catalog)
    }

    fn by_name(&self, name: &str) -> Option<&Target> {
        let start = Target { name: name.to_owned(), provider: None };
        self.nodes.range(start..).next().map(|(key, _)| key).filter(|key| key.name == name)
    }

    fn group_targets(&self, group: &crate::api::Group) -> Vec<Target> {
        let mut visited = BTreeSet::from([group.name.as_str()]);
        let mut pending = group.all.iter().rev().map(String::as_str).collect::<Vec<_>>();
        let mut targets = Vec::new();

        while let Some(name) = pending.pop() {
            if !visited.insert(name) {
                continue;
            }
            if let Some(subgroup) = self.groups.iter().find(|group| group.name == name) {
                pending.extend(subgroup.all.iter().rev().map(String::as_str));
            } else if let Some(target) = self.by_name(name).filter(|target| self.testable(target)) {
                targets.push(target.clone());
            }
        }
        targets
    }

    fn testable(&self, target: &Target) -> bool {
        self.nodes.get(target).is_some_and(|node| {
            !matches!(node.kind.as_str(), "Compatible" | "Reject" | "RejectDrop" | "Pass")
        })
    }
}

#[derive(Default)]
struct App {
    page: Page,
    help_offset: Option<usize>,
    delete_confirmation: Option<String>,
    messages: [Notice; Page::COUNT],
    catalog: Catalog,
    group: ListState,
    node: ListState,
    node_focus: bool,
    measured: BTreeMap<Target, u64>,
    testing: Option<SpeedTest>,
    speed_result: Option<SpeedResult>,
    test_concurrency: usize,
    overview: Overview,
    rules: Vec<Rule>,
    connections: Connections,
    logs: VecDeque<LogEntry>,
    rules_cursor: TableState,
    conn_cursor: TableState,
    log_cursor: ListState,
    profiles: Vec<ProfileInfo>,
    profile_cursor: ListState,
    system_mode: bool,
    follow_logs: bool,
}

#[derive(Default)]
struct Notice {
    text: String,
    success: bool,
}

impl From<String> for Notice {
    fn from(text: String) -> Self {
        Self { text, success: false }
    }
}

impl From<&str> for Notice {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

#[derive(Clone, Copy, Default)]
struct Rate { up: u64, down: u64 }

#[derive(Default)]
struct Overview {
    version: String,
    snapshot: Connections,
    last: Option<Instant>,
    history: VecDeque<Rate>,
}

impl Overview {
    fn receive(&mut self, snapshot: Connections, version: Option<String>) {
        if let Some(version) = version { self.version = version; }
        let now = Instant::now();
        if let Some(last) = self.last {
            let elapsed = now.duration_since(last).as_secs_f64();
            if elapsed > 3.0 || snapshot.upload_total < self.snapshot.upload_total
                || snapshot.download_total < self.snapshot.download_total
            {
                self.history.clear();
            } else if elapsed > 0.0 {
                if self.history.len() == 60 { self.history.pop_front(); }
                self.history.push_back(Rate {
                    up: ((snapshot.upload_total - self.snapshot.upload_total) as f64 / elapsed).round() as u64,
                    down: ((snapshot.download_total - self.snapshot.download_total) as f64 / elapsed).round() as u64,
                });
            }
        }
        self.last = Some(now);
        self.snapshot = snapshot;
    }
}

struct SpeedResult {
    notice: Notice,
    expires: Instant,
}

struct SpeedTest {
    scope: String,
    receive: mpsc::Receiver<(Target, u64)>,
    completed: usize,
    failed: usize,
    total: usize,
}

pub(crate) enum Backend {
    User { service: Service, client: Mutex<Option<(Endpoint, Arc<Client>)>> },
    System(Arc<Client>),
}

impl Backend {
    pub(crate) fn user(service: Service) -> Self {
        Self::User { service, client: Mutex::new(None) }
    }

    pub(crate) fn system(client: Client) -> Self {
        Self::System(Arc::new(client))
    }

    fn client(&self) -> Result<Arc<Client>> {
        match self {
            Self::System(client) => Ok(Arc::clone(client)),
            Self::User { service, client } => {
                let endpoint = service.profile()?.context("Import a configuration with clashpt profile import")?.endpoint;
                let mut stored = client.lock().map_err(|_| anyhow!("Mihomo client lock is unavailable"))?;
                if let Some((current, client)) = &*stored && current == &endpoint {
                    return Ok(Arc::clone(client));
                }
                let client = Arc::new(Client::new(endpoint.clone())?);
                *stored = Some((endpoint, Arc::clone(&client)));
                Ok(client)
            }
        }
    }

    fn service(&self) -> Result<&Service> {
        match self {
            Self::User { service, .. } => Ok(service),
            Self::System(_) => Err(anyhow!("Use sudo clashpt system apply to change the system profile")),
        }
    }
}

enum Action {
    Refresh,
    ManualRefresh,
    SelectNode(String, String),
    UseProfile(String),
    DeleteProfile(String),
}

enum Data {
    Overview(Connections, Option<String>),
    Proxies(Catalog),
    Rules(Vec<Rule>),
    Conns(Connections),
    Logs(Vec<LogEntry>),
    Profiles(Vec<ProfileInfo>, Option<ProfileChange>),
}

struct ProfileChange {
    message: String,
    active_changed: bool,
}

struct Reply {
    data: Result<Data>,
    switched: bool,
    refreshed: bool,
}

fn request(backend: &Arc<Backend>, page: Page, action: Action, version: bool) -> mpsc::Receiver<Reply> {
    let (send, receive) = mpsc::channel();
    let backend = Arc::clone(backend);
    thread::spawn(move || {
        let switched = matches!(action, Action::SelectNode(..));
        let refreshed = matches!(action, Action::ManualRefresh);
        let data = (|| {
            if page == Page::Profiles {
                let service = backend.service()?;
                let changed = match action {
                    Action::UseProfile(name) if service.activate(&name)? => Some(ProfileChange {
                        message: format!("Active profile: {name}"),
                        active_changed: true,
                    }),
                    Action::DeleteProfile(name) => {
                        let result = service.delete_profile(&name)?;
                        Some(ProfileChange {
                            message: result.message(&name), active_changed: result.active_changed,
                        })
                    }
                    _ => None,
                };
                return Ok(Data::Profiles(service.list_profiles()?, changed));
            }

            let client = backend.client()?;
            if let Action::SelectNode(group, node) = action {
                client.select(&group, &node)?;
            }
            Ok(match page {
                Page::Overview => {
                    let snapshot = client.connections()?;
                    let version = version.then(|| client.version().ok()).flatten().map(|v| v.version);
                    Data::Overview(snapshot, version)
                }
                Page::Proxies => Data::Proxies(Catalog::load(&client)?),
                Page::Rules => Data::Rules(client.rules()?.rules),
                Page::Conns => Data::Conns(client.connections()?),
                Page::Logs => Data::Logs(client.logs(Duration::from_millis(3500))?),
                Page::Profiles => unreachable!(),
            })
        })();
        let _ = send.send(Reply { data, switched, refreshed });
    });
    receive
}

pub fn run(backend: Backend, config: Option<&Path>, gateway_workers: Option<NonZeroUsize>) -> Result<()> {
    let settings = Settings::load(config)?;
    let mut concurrency = settings.latency_test.concurrency.get();
    if let Some(workers) = gateway_workers {
        concurrency = concurrency.min(workers.get());
    }
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    let result = run_app(&mut terminal, Arc::new(backend), &settings, concurrency);
    let _ = terminal.show_cursor();
    result
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn run_app(terminal: &mut DefaultTerminal, backend: Arc<Backend>, settings: &Settings, test_concurrency: usize) -> Result<()> {
    let mut app = App {
        follow_logs: true, test_concurrency, system_mode: matches!(backend.as_ref(), Backend::System(_)),
        ..App::default()
    };
    let mut pending: [Option<mpsc::Receiver<Reply>>; Page::COUNT] = Default::default();
    let mut due: [Option<Instant>; Page::COUNT] = Default::default();
    due[app.page.index()] = Some(Instant::now());
    let mut redraw = true;
    let mut changing_profiles = false;
    let mut quit_after_profile_change = false;
    loop {
        for page in Page::ALL {
            let index = page.index();
            if let Some(receiver) = &pending[index] {
                let reply = match receiver.try_recv() {
                    Ok(reply) => reply,
                    Err(mpsc::TryRecvError::Empty) => continue,
                    Err(mpsc::TryRecvError::Disconnected) => Reply {
                        data: Err(anyhow!("Mihomo request ended unexpectedly")), switched: false, refreshed: false,
                    },
                };
                pending[index] = None;
                if page == Page::Profiles && changing_profiles {
                    changing_profiles = false;
                    if quit_after_profile_change { return Ok(()); }
                    if matches!(&reply.data, Ok(Data::Profiles(_, Some(change))) if change.active_changed) {
                        pending = Default::default();
                        due = Default::default();
                        app.reset_runtime();
                        if app.page != Page::Profiles {
                            due[app.page.index()] = Some(Instant::now());
                        }
                        redraw = true;
                    }
                }
                let failed = reply.data.is_err();
                app.receive(page, reply);
                let interval = if failed { Duration::from_secs(2) } else { page.interval().unwrap_or_default() };
                due[index] = page.interval().map(|_| Instant::now() + interval);
                redraw |= page == app.page;
            }
        }
        redraw |= app.receive_speed();

        let page = app.page;
        let index = page.index();
        if !changing_profiles && pending[index].is_none() && due[index].is_some_and(|time| time <= Instant::now()) {
            pending[index] = Some(request(&backend, page, Action::Refresh, app.overview.version.is_empty()));
            due[index] = None;
        }
        if redraw {
            terminal.draw(|frame| views::draw(&mut app, frame, settings))?;
            redraw = false;
        }
        let busy = pending.iter().any(Option::is_some) || due[index].is_some()
            || app.testing.is_some() || (page == Page::Proxies && app.speed_result.is_some());
        if busy && !event::poll(Duration::from_millis(70))? { continue; }

        match event::read()? {
            Event::Resize(_, _) => redraw = true,
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                let interrupt = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                let quit = interrupt || (app.help_offset.is_none() && matches!(key.code, KeyCode::Esc | KeyCode::Char('q')));
                if quit && changing_profiles {
                    quit_after_profile_change = true;
                    continue;
                }
                if interrupt {
                    return Ok(());
                }
                if app.delete_confirmation.is_some() {
                    match key.code {
                        KeyCode::Char('y') if key.kind == KeyEventKind::Press => {
                            let name = app.delete_confirmation.take().unwrap();
                            app.testing = None;
                            changing_profiles = true;
                            app.messages[index] = format!("Deleting {name}…").into();
                            pending[index] = Some(request(&backend, page, Action::DeleteProfile(name), false));
                        }
                        KeyCode::Esc | KeyCode::Char('n' | 'q') => app.delete_confirmation = None,
                        _ => continue,
                    }
                    redraw = true;
                    continue;
                }
                if let Some(offset) = &mut app.help_offset {
                    match key.code {
                        KeyCode::Char('?') | KeyCode::Esc | KeyCode::Enter if key.kind == KeyEventKind::Press => {
                            app.help_offset = None;
                        }
                        KeyCode::Up | KeyCode::Char('k') => *offset = offset.saturating_sub(1),
                        KeyCode::Down | KeyCode::Char('j') => *offset = offset.saturating_add(1),
                        KeyCode::PageUp => *offset = offset.saturating_sub(5),
                        KeyCode::PageDown => *offset = offset.saturating_add(5),
                        _ => continue,
                    }
                    redraw = true;
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('?') if key.kind == KeyEventKind::Press => app.help_offset = Some(0),
                    KeyCode::Char(n @ '1'..='6') => {
                        app.page = Page::ALL[(n as u8 - b'1') as usize];
                        if app.page != page {
                            due[app.page.index()] = Some(Instant::now());
                            if app.page == Page::Overview { app.overview.last = None; }
                        }
                    }
                    KeyCode::Tab if page == Page::Proxies => app.node_focus = !app.node_focus,
                    KeyCode::Left | KeyCode::Char('h') if page == Page::Proxies => app.node_focus = false,
                    KeyCode::Right | KeyCode::Char('l') if page == Page::Proxies => app.node_focus = true,
                    KeyCode::Up | KeyCode::Char('k') => app.navigate(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.navigate(1),
                    KeyCode::PageUp => app.navigate(-12),
                    KeyCode::PageDown => app.navigate(12),
                    KeyCode::End if page == Page::Logs => {
                        app.follow_logs = true;
                        app.log_cursor.select(app.logs.len().checked_sub(1));
                    }
                    KeyCode::Char('r') if pending[index].is_none() && !changing_profiles => {
                        pending[index] = Some(request(&backend, page, Action::ManualRefresh, app.overview.version.is_empty()));
                        due[index] = None;
                        app.messages[index] = "Refreshing…".into();
                    }
                    KeyCode::Char('t') if page == Page::Proxies && !changing_profiles => {
                        let targets = app.catalog.nodes.keys().filter(|key| app.catalog.testable(key)).cloned().collect();
                        app.start_speed(&backend, "All nodes".into(), targets);
                    }
                    KeyCode::Char('g') if page == Page::Proxies && !changing_profiles => {
                        if let Some(group) = app.current_group() {
                            let scope = group.name.clone();
                            let targets = app.catalog.group_targets(group);
                            app.start_speed(&backend, scope, targets);
                        } else if app.testing.is_none() {
                            app.messages[index] = "Choose a strategy group to test its nodes".into();
                        }
                    }
                    KeyCode::Char('d') if page == Page::Proxies && !changing_profiles => {
                        if let Some(target) = app.current_target().filter(|key| app.catalog.testable(key)).cloned() {
                            app.start_speed(&backend, target.name.clone(), vec![target]);
                        } else if app.testing.is_none() {
                            app.messages[index] = "Choose a testable node".into();
                        }
                    }
                    KeyCode::Enter if page == Page::Proxies && !app.node_focus => app.node_focus = true,
                    KeyCode::Enter if page == Page::Proxies && pending[index].is_none() && !changing_profiles => {
                        if let Some((group, node)) = app.selection() {
                            pending[index] = Some(request(&backend, page, Action::SelectNode(group, node), false));
                            app.messages[index] = "Switching node…".into();
                        } else if app.group.selected() == Some(0) {
                            app.messages[index] = "Choose a strategy group to switch nodes".into();
                        }
                    }
                    KeyCode::Delete | KeyCode::Char('x') if page == Page::Profiles
                        && pending[index].is_none() && pending[Page::Proxies.index()].is_none() =>
                    {
                        app.delete_confirmation = app.profile_cursor.selected()
                            .and_then(|i| app.profiles.get(i)).map(|profile| profile.name.clone());
                    }
                    KeyCode::Enter if page == Page::Profiles && pending[index].is_none()
                        && pending[Page::Proxies.index()].is_none() =>
                    {
                        if let Some(profile) = app.profile_cursor.selected().and_then(|i| app.profiles.get(i)) {
                            if !profile.active {
                                let name = profile.name.clone();
                                app.testing = None;
                                changing_profiles = true;
                                app.messages[index] = format!("Switching to {name}…").into();
                                pending[index] = Some(request(&backend, page, Action::UseProfile(name), false));
                            }
                        }
                    }
                    _ => continue,
                }
                redraw = true;
            }
            _ => {}
        }
    }
}

impl App {
    fn reset_runtime(&mut self) {
        self.catalog = Catalog::default();
        self.group = ListState::default();
        self.node = ListState::default();
        self.node_focus = false;
        self.measured.clear();
        self.testing = None;
        self.speed_result = None;
        self.overview = Overview::default();
        self.rules.clear();
        self.rules_cursor = TableState::default();
        self.connections = Connections::default();
        self.conn_cursor = TableState::default();
        self.logs.clear();
        self.log_cursor = ListState::default();
        self.follow_logs = true;
        for page in Page::ALL {
            if page != Page::Profiles {
                self.messages[page.index()] = Notice::default();
            }
        }
    }

    fn current_group(&self) -> Option<&crate::api::Group> {
        self.group.selected()?.checked_sub(1).and_then(|index| self.catalog.groups.get(index))
    }

    fn selection(&self) -> Option<(String, String)> {
        let group = self.current_group()?;
        let node = group.all.get(self.node.selected()?)?;
        Some((group.name.clone(), node.clone()))
    }

    fn current_target(&self) -> Option<&Target> {
        let index = self.node.selected()?;
        if let Some(group) = self.current_group() {
            self.catalog.by_name(group.all.get(index)?)
        } else {
            self.catalog.nodes.keys().nth(index)
        }
    }

    fn reset_node(&mut self) {
        let selected = match self.current_group() {
            Some(group) if !group.all.is_empty() => Some(group.all.iter().position(|node| node == &group.now).unwrap_or(0)),
            Some(_) => None,
            None => (!self.catalog.nodes.is_empty()).then_some(0),
        };
        self.node.select(selected);
        *self.node.offset_mut() = 0;
    }

    fn receive(&mut self, page: Page, reply: Reply) {
        let index = page.index();
        let data = match reply.data {
            Ok(data) => data,
            Err(error) => {
                if page == Page::Overview { self.overview.last = None; }
                self.messages[index] = error.to_string().into();
                return;
            }
        };
        let (text, success) = if reply.switched {
            ("Node switched", true)
        } else if reply.refreshed {
            ("Refreshed", true)
        } else {
            ("", false)
        };
        let mut message = Notice { text: text.into(), success };
        match data {
            Data::Overview(snapshot, version) => self.overview.receive(snapshot, version),
            Data::Proxies(catalog) => {
                let old_group = self.current_group().map(|group| group.name.clone());
                let old_node = self.node.selected();
                let index = match (self.group.selected(), old_group) {
                    (Some(0), _) => 0,
                    (_, Some(name)) => catalog.groups.iter().position(|group| group.name == name).map_or(0, |index| index + 1),
                    _ => catalog.groups.iter().position(|group| group.kind == "Selector" && group.name != "GLOBAL")
                        .or_else(|| (!catalog.groups.is_empty()).then_some(0)).map_or(0, |index| index + 1),
                };
                self.catalog = catalog;
                self.measured.retain(|target, _| self.catalog.nodes.contains_key(target));
                self.group.select(Some(index));
                self.reset_node();
                if !reply.switched && let Some(node) = old_node {
                    let count = self.current_group().map_or(self.catalog.nodes.len(), |group| group.all.len());
                    self.node.select(count.checked_sub(1).map(|last| node.min(last)));
                }
                if self.catalog.nodes.is_empty() && self.catalog.groups.is_empty() {
                    message.text = "No proxies in the active configuration".into();
                }
            }
            Data::Rules(rules) => {
                self.rules = rules;
                self.rules_cursor.select(cursor(self.rules_cursor.selected(), self.rules.len(), 0));
            }
            Data::Conns(connections) => {
                let state = &mut self.conn_cursor;
                let focused = state.selected().and_then(|i| self.connections.connections.get(i));
                if let Some(focused) = focused && let Some(i) = connections.connections.iter().position(|conn| conn.id == focused.id) {
                    state.select(Some(i));
                }
                self.connections = connections;
                state.select(cursor(state.selected(), self.connections.connections.len(), 0));
            }
            Data::Profiles(profiles, changed) => {
                let selected = self.profile_cursor.selected()
                    .and_then(|i| self.profiles.get(i)).map(|profile| &profile.name);
                let current = selected.and_then(|name| profiles.iter().position(|profile| &profile.name == name))
                    .or_else(|| profiles.iter().position(|profile| profile.active));
                self.profile_cursor.select(cursor(current, profiles.len(), 0));
                self.profiles = profiles;
                if let Some(change) = changed {
                    message = Notice { text: change.message, success: true };
                }
            }
            Data::Logs(logs) => {
                let dropped = (self.logs.len() + logs.len()).saturating_sub(400);
                self.logs.drain(..dropped);
                self.logs.extend(logs);
                let state = &mut self.log_cursor;
                if self.follow_logs {
                    state.select(self.logs.len().checked_sub(1));
                } else if let Some(selected) = state.selected() {
                    state.select(Some(selected.saturating_sub(dropped)));
                }
            }
        }
        self.messages[index] = message;
    }

    fn navigate(&mut self, delta: isize) {
        match self.page {
            Page::Overview => {}
            Page::Proxies if self.node_focus => {
                let count = self.current_group().map_or(self.catalog.nodes.len(), |group| group.all.len());
                self.node.select(cursor(self.node.selected(), count, delta));
            }
            Page::Proxies => {
                let before = self.group.selected();
                self.group.select(cursor(before, self.catalog.groups.len() + 1, delta));
                if self.group.selected() != before { self.reset_node(); }
            }
            Page::Rules => self.rules_cursor.select(cursor(self.rules_cursor.selected(), self.rules.len(), delta)),
            Page::Conns => self.conn_cursor.select(cursor(self.conn_cursor.selected(), self.connections.connections.len(), delta)),
            Page::Profiles => self.profile_cursor.select(cursor(self.profile_cursor.selected(), self.profiles.len(), delta)),
            Page::Logs => {
                self.log_cursor.select(cursor(self.log_cursor.selected(), self.logs.len(), delta));
                self.follow_logs = self.log_cursor.selected() == self.logs.len().checked_sub(1);
            }
        }
    }

    fn start_speed(&mut self, backend: &Backend, scope: String, targets: Vec<Target>) {
        if self.testing.is_some() { return; }
        if targets.is_empty() {
            self.messages[1] = format!("{scope} has no testable nodes").into();
            return;
        }
        let client = match backend.client() {
            Ok(client) => client,
            Err(error) => {
                self.messages[Page::Proxies.index()] = error.to_string().into();
                return;
            }
        };
        let total = targets.len();
        let targets = Arc::new(targets);
        let next = Arc::new(AtomicUsize::new(0));
        let (send, receive) = mpsc::channel();
        for _ in 0..total.min(self.test_concurrency) {
            let (targets, next, send, client) = (Arc::clone(&targets), Arc::clone(&next), send.clone(), Arc::clone(&client));
            thread::spawn(move || {
                while let Some(target) = targets.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let delay = client.delay(target).map_or(0, |result| result.delay);
                    if send.send((target.clone(), delay)).is_err() { break; }
                }
            });
        }
        self.messages[Page::Proxies.index()] = Notice::default();
        self.speed_result = None;
        self.testing = Some(SpeedTest { scope, receive, completed: 0, failed: 0, total });
    }

    fn status(&self) -> (Cow<'_, str>, bool) {
        if self.page == Page::Proxies {
            if let Some(test) = &self.testing {
                return (format!("Testing latency · {} • {}/{}", test.scope, test.completed, test.total).into(), false);
            }
            if let Some(result) = &self.speed_result {
                return (result.notice.text.as_str().into(), result.notice.success);
            }
        }
        let notice = &self.messages[self.page.index()];
        (notice.text.as_str().into(), notice.success)
    }

    fn receive_speed(&mut self) -> bool {
        if self.speed_result.as_ref().is_some_and(|result| Instant::now() >= result.expires) {
            self.speed_result = None;
            return self.page == Page::Proxies;
        }
        let Some(test) = &mut self.testing else { return false };
        let previous = test.completed;
        loop {
            match test.receive.try_recv() {
                Ok((key, delay)) => {
                    self.measured.insert(key, delay);
                    test.completed += 1;
                    test.failed += usize::from(delay == 0);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let mut text = format!("Latency test finished · {} • {}/{}", test.scope, test.completed, test.total);
                    if test.failed > 0 {
                        text.push_str(&format!(" ({} failed)", test.failed));
                    }
                    self.speed_result = Some(SpeedResult {
                        notice: Notice { text, success: test.failed == 0 && test.completed == test.total },
                        expires: Instant::now() + Duration::from_secs(3),
                    });
                    self.testing = None;
                    return self.page == Page::Proxies;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        test.completed != previous && self.page == Page::Proxies
    }
}

fn cursor(selected: Option<usize>, count: usize, delta: isize) -> Option<usize> {
    count.checked_sub(1).map(|last| selected.unwrap_or(0).saturating_add_signed(delta).min(last))
}
