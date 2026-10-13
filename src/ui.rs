use crate::{
    backend,
    model::{status_label, Ticket},
    watch::TicketWatcher,
};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, Tabs, Wrap,
    },
    Frame, Terminal,
};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Kanban,
    List,
    Table,
    Graph,
}

impl View {
    fn next(self, backwards: bool) -> Self {
        match (self, backwards) {
            (Self::Kanban, false) => Self::List,
            (Self::List, false) => Self::Table,
            (Self::Table, false) => Self::Graph,
            (Self::Graph, false) => Self::Kanban,
            (Self::Kanban, true) => Self::Graph,
            (Self::List, true) => Self::Kanban,
            (Self::Table, true) => Self::List,
            (Self::Graph, true) => Self::Table,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Kanban => 0,
            Self::List => 1,
            Self::Table => 2,
            Self::Graph => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Normal,
    Filter,
    Create,
    Help,
}

#[derive(Default)]
struct CreateForm {
    title: String,
    description: String,
    field: usize,
}

struct App {
    tickets_dir: PathBuf,
    tickets: Vec<Ticket>,
    view: View,
    mode: Mode,
    filter: String,
    input: String,
    create: CreateForm,
    show_closed: bool,
    show_detail: bool,
    moving: bool,
    active_column: usize,
    selected: usize,
    message: String,
    should_quit: bool,
}

impl App {
    fn new(tickets_dir: PathBuf, tickets: Vec<Ticket>, compact: bool) -> Self {
        Self {
            tickets_dir,
            tickets,
            view: if compact { View::List } else { View::Kanban },
            mode: Mode::Normal,
            filter: String::new(),
            input: String::new(),
            create: CreateForm::default(),
            show_closed: false,
            show_detail: !compact,
            moving: false,
            active_column: 0,
            selected: 0,
            message: "Ready".to_owned(),
            should_quit: false,
        }
    }

    fn columns(&self) -> Vec<String> {
        let mut statuses = vec!["open".to_owned(), "in_progress".to_owned()];
        let mut custom = self
            .tickets
            .iter()
            .map(|ticket| ticket.status.clone())
            .filter(|status| !["open", "in_progress", "closed"].contains(&status.as_str()))
            .collect::<Vec<_>>();
        custom.sort();
        custom.dedup();
        statuses.extend(custom);
        if self.show_closed {
            statuses.push("closed".to_owned());
        }
        statuses
    }

    fn active_status(&self) -> String {
        let columns = self.columns();
        columns
            .get(self.active_column.min(columns.len().saturating_sub(1)))
            .cloned()
            .unwrap_or_else(|| "open".to_owned())
    }

    fn matches_filter(&self, ticket: &Ticket) -> bool {
        if !self.show_closed && ticket.status == "closed" {
            return false;
        }
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        ticket.id.to_lowercase().contains(&needle)
            || ticket.title.to_lowercase().contains(&needle)
            || ticket.status.to_lowercase().contains(&needle)
            || ticket.kind.to_lowercase().contains(&needle)
            || ticket.assignee.to_lowercase().contains(&needle)
            || ticket
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(&needle))
    }

    fn visible_for_status(&self, status: &str) -> Vec<&Ticket> {
        self.tickets
            .iter()
            .filter(|ticket| ticket.status == status && self.matches_filter(ticket))
            .collect()
    }

    fn visible_flat(&self) -> Vec<&Ticket> {
        let columns = self.columns();
        let mut tickets = self
            .tickets
            .iter()
            .filter(|ticket| self.matches_filter(ticket))
            .collect::<Vec<_>>();
        tickets.sort_by(|left, right| {
            let left_column = columns
                .iter()
                .position(|status| status == &left.status)
                .unwrap_or(0);
            let right_column = columns
                .iter()
                .position(|status| status == &right.status)
                .unwrap_or(0);
            left_column
                .cmp(&right_column)
                .then(left.priority.cmp(&right.priority))
                .then(left.id.cmp(&right.id))
        });
        tickets
    }

    fn visible_graph(&self) -> Vec<&Ticket> {
        graph_order(&self.tickets)
            .into_iter()
            .filter(|ticket| self.matches_filter(ticket))
            .collect()
    }

    fn selected_ticket(&self) -> Option<&Ticket> {
        match self.view {
            View::Kanban => self
                .visible_for_status(&self.active_status())
                .get(self.selected)
                .copied(),
            View::List | View::Table => self.visible_flat().get(self.selected).copied(),
            View::Graph => self.visible_graph().get(self.selected).copied(),
        }
    }

    fn selection_id(&self) -> Option<String> {
        self.selected_ticket().map(|ticket| ticket.id.clone())
    }

    fn selected_count(&self) -> usize {
        match self.view {
            View::Kanban => self.visible_for_status(&self.active_status()).len(),
            View::List | View::Table => self.visible_flat().len(),
            View::Graph => self.visible_graph().len(),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.selected_count();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
    }

    fn refresh(&mut self, selected_id: Option<&str>, status_hint: Option<&str>) -> bool {
        match backend::load(&self.tickets_dir) {
            Ok(tickets) => {
                let changed = self.tickets != tickets;
                self.tickets = tickets;
                if let Some(status) = status_hint {
                    if let Some(index) = self.columns().iter().position(|column| column == status) {
                        self.active_column = index;
                    }
                }
                if let Some(id) = selected_id {
                    let index = match self.view {
                        View::Kanban => self
                            .visible_for_status(&self.active_status())
                            .iter()
                            .position(|ticket| ticket.id == id),
                        View::List | View::Table => self
                            .visible_flat()
                            .iter()
                            .position(|ticket| ticket.id == id),
                        View::Graph => self
                            .visible_graph()
                            .iter()
                            .position(|ticket| ticket.id == id),
                    };
                    if let Some(index) = index {
                        self.selected = index;
                    }
                }
                self.selected = self.selected.min(self.selected_count().saturating_sub(1));
                changed
            }
            Err(error) => {
                self.message = format!("Refresh failed: {error:#}");
                false
            }
        }
    }

    fn change_status(&mut self, next_status: &str) {
        let Some(ticket) = self.selected_ticket() else {
            self.message = "No ticket selected".to_owned();
            return;
        };
        let id = ticket.id.clone();
        match backend::set_status(&self.tickets_dir, &id, next_status) {
            Ok(()) => {
                self.message = format!("Moved {id} to {}", status_label(next_status));
                self.refresh(Some(&id), Some(next_status));
            }
            Err(error) => self.message = format!("Could not update {id}: {error:#}"),
        }
    }

    fn move_to_neighbor(&mut self, direction: isize) {
        let columns = self.columns();
        if columns.get(self.active_column).is_none() {
            return;
        }
        let next = self.active_column as isize + direction;
        if next < 0 || next >= columns.len() as isize {
            self.message = "No status column in that direction".to_owned();
            return;
        }
        let target = columns[next as usize].clone();
        if self.selected_ticket().is_some() {
            self.change_status(&target);
        } else {
            self.active_column = next as usize;
            self.selected = 0;
            self.message = format!("{} column", status_label(&target));
        }
    }

    fn begin_create(&mut self) {
        self.create = CreateForm::default();
        self.mode = Mode::Create;
        self.message.clear();
    }

    fn submit_create(&mut self) {
        let title = self.create.title.trim().to_owned();
        if title.is_empty() {
            self.message = "A ticket needs a title".to_owned();
            return;
        }
        match backend::create(&self.tickets_dir, &title, &self.create.description) {
            Ok(id) => {
                self.mode = Mode::Normal;
                self.view = View::List;
                self.show_closed = false;
                self.filter.clear();
                self.message = if id.is_empty() {
                    format!("Created {title}")
                } else {
                    format!("Created {id}")
                };
                self.refresh((!id.is_empty()).then_some(id.as_str()), Some("open"));
            }
            Err(error) => self.message = format!("Could not create ticket: {error:#}"),
        }
    }

    fn filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
                self.filter.clear();
                self.selected = 0;
            }
            KeyCode::Enter => {
                self.filter = self.input.trim().to_owned();
                self.mode = Mode::Normal;
                self.selected = 0;
                self.message = if self.filter.is_empty() {
                    "Filter cleared".to_owned()
                } else {
                    format!("Filter: {}", self.filter)
                };
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(character);
            }
            _ => {}
        }
    }

    fn create_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.message = "Create cancelled".to_owned();
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
                self.create.field = 1 - self.create.field;
            }
            KeyCode::Enter if self.create.field == 0 => self.create.field = 1,
            KeyCode::Enter => self.submit_create(),
            KeyCode::Backspace => {
                let value = if self.create.field == 0 {
                    &mut self.create.title
                } else {
                    &mut self.create.description
                };
                value.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let value = if self.create.field == 0 {
                    &mut self.create.title
                } else {
                    &mut self.create.description
                };
                value.push(character);
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match self.mode {
            Mode::Filter => return self.filter_key(key),
            Mode::Create => return self.create_key(key),
            Mode::Help => {
                self.mode = Mode::Normal;
                return;
            }
            Mode::Normal => {}
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc if self.moving => {
                self.moving = false;
                self.message = "Move mode cancelled".to_owned();
            }
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('K') | KeyCode::Tab => {
                self.view = self.view.next(key.modifiers.contains(KeyModifiers::SHIFT));
                self.selected = 0;
            }
            KeyCode::BackTab => {
                self.view = self.view.next(true);
                self.selected = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('h') | KeyCode::Left if self.view == View::Kanban => {
                if self.moving {
                    self.move_to_neighbor(-1);
                } else {
                    self.active_column = self.active_column.saturating_sub(1);
                    self.selected = self.selected.min(self.selected_count().saturating_sub(1));
                }
            }
            KeyCode::Char('l') | KeyCode::Right if self.view == View::Kanban => {
                if self.moving {
                    self.move_to_neighbor(1);
                } else {
                    self.active_column = (self.active_column + 1).min(self.columns().len() - 1);
                    self.selected = self.selected.min(self.selected_count().saturating_sub(1));
                }
            }
            KeyCode::Char('v') if self.view == View::Kanban => {
                self.moving = !self.moving;
                self.message = if self.moving {
                    "Move mode: use h/l or arrows, Esc to cancel".to_owned()
                } else {
                    "Move mode off".to_owned()
                };
            }
            KeyCode::Char('a') => self.begin_create(),
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                self.input = self.filter.clone();
            }
            KeyCode::Char('C') => {
                self.show_closed = !self.show_closed;
                self.selected = 0;
                self.message = if self.show_closed {
                    "Closed tickets visible".to_owned()
                } else {
                    "Closed tickets hidden".to_owned()
                };
            }
            KeyCode::Char('d') | KeyCode::Enter => self.show_detail = !self.show_detail,
            KeyCode::Char('r') => {
                let selected = self.selection_id();
                self.refresh(selected.as_deref(), None);
                self.message = format!("Loaded {} tickets", self.tickets.len());
            }
            KeyCode::Char('x') => self.change_status("closed"),
            KeyCode::Char('o') => self.change_status("open"),
            KeyCode::Char('s') => {
                if let Some(ticket) = self.selected_ticket() {
                    let next = match ticket.status.as_str() {
                        "open" => "in_progress",
                        "in_progress" => "closed",
                        _ => "open",
                    };
                    self.change_status(next);
                }
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }
}

fn graph_order(tickets: &[Ticket]) -> Vec<&Ticket> {
    let indexes = tickets
        .iter()
        .enumerate()
        .map(|(index, ticket)| (ticket.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut prerequisites = vec![0; tickets.len()];
    let mut dependents = vec![Vec::new(); tickets.len()];
    for (index, ticket) in tickets.iter().enumerate() {
        let mut resolved = Vec::new();
        for dependency in &ticket.dependencies {
            if let Some(dependency_index) = indexes.get(dependency.as_str()).copied() {
                if !resolved.contains(&dependency_index) {
                    resolved.push(dependency_index);
                    dependents[dependency_index].push(index);
                }
            }
        }
        prerequisites[index] = resolved.len();
    }

    let mut ready = BinaryHeap::new();
    for (index, ticket) in tickets.iter().enumerate() {
        if prerequisites[index] == 0 {
            ready.push(Reverse((ticket.priority, ticket.id.clone(), index)));
        }
    }
    let mut emitted = vec![false; tickets.len()];
    let mut ordered = Vec::with_capacity(tickets.len());
    while let Some(Reverse((_, _, index))) = ready.pop() {
        emitted[index] = true;
        ordered.push(&tickets[index]);
        for dependent in &dependents[index] {
            prerequisites[*dependent] -= 1;
            if prerequisites[*dependent] == 0 {
                let ticket = &tickets[*dependent];
                ready.push(Reverse((ticket.priority, ticket.id.clone(), *dependent)));
            }
        }
    }
    // Keep cyclic tickets visible after all nodes that can be ordered.
    if ordered.len() < tickets.len() {
        let mut remaining = (0..tickets.len())
            .filter(|index| !emitted[*index])
            .collect::<Vec<_>>();
        remaining.sort_by(|left, right| {
            tickets[*left]
                .priority
                .cmp(&tickets[*right].priority)
                .then(tickets[*left].id.cmp(&tickets[*right].id))
        });
        ordered.extend(remaining.into_iter().map(|index| &tickets[index]));
    }
    ordered
}

fn dependency_state(ticket: &Ticket, tickets_by_id: &HashMap<&str, &Ticket>) -> &'static str {
    let mut waiting = false;
    for dependency in &ticket.dependencies {
        match tickets_by_id.get(dependency.as_str()) {
            Some(candidate) if candidate.status == "closed" => {}
            Some(_) => waiting = true,
            None => return "MISSING",
        }
    }
    if waiting {
        "WAITING"
    } else {
        "READY"
    }
}

fn dependency_color(status: &str) -> Color {
    match status {
        "closed" => Color::Green,
        "in_progress" => Color::Yellow,
        "open" => Color::Cyan,
        _ => Color::DarkGray,
    }
}

pub fn run(tickets_dir: PathBuf, tickets: Vec<Ticket>, compact: bool) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(tickets_dir, tickets, compact);
    let mut watcher = match TicketWatcher::new(&app.tickets_dir) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            app.message = format!("Auto-refresh unavailable: {error}");
            None
        }
    };
    let result = event_loop(&mut terminal, &mut app, watcher.as_mut());

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    watcher: Option<&mut TicketWatcher>,
) -> Result<()> {
    let mut watcher = watcher;
    let mut refresh_at: Option<Instant> = None;
    while !app.should_quit {
        terminal.draw(|frame| draw(frame, app))?;
        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }
        if watcher
            .as_deref_mut()
            .is_some_and(TicketWatcher::tickets_changed)
        {
            // `tk` often saves through a temporary file and rename. Debounce
            // the event burst so one command causes a single reload.
            refresh_at = Some(Instant::now() + Duration::from_millis(160));
        }
        if refresh_at.is_some_and(|deadline| Instant::now() >= deadline) {
            refresh_at = None;
            let selected = app.selection_id();
            if app.refresh(selected.as_deref(), None) {
                app.message = format!("Tickets updated · {} loaded", app.tickets.len());
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());
    draw_header(frame, chunks[0], app);
    draw_toolbar(frame, chunks[1], app);

    if app.show_detail && frame.area().width >= 96 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
            .split(chunks[2]);
        draw_board(frame, body[0], app);
        draw_detail(frame, body[1], app);
    } else if app.show_detail && frame.area().height >= 24 {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(chunks[2]);
        draw_board(frame, body[0], app);
        draw_detail(frame, body[1], app);
    } else {
        draw_board(frame, chunks[2], app);
    }
    draw_footer(frame, chunks[3], app);

    match app.mode {
        Mode::Create => draw_create_dialog(frame, app),
        Mode::Filter => draw_filter_dialog(frame, app),
        Mode::Help => draw_help(frame),
        Mode::Normal => {}
    }
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            "TICKET BOARD",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.tickets_dir.display().to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("herdr-ticket")),
        area,
    );
}

fn draw_toolbar(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let tabs = Tabs::new(vec![
        Line::from("Kanban"),
        Line::from("List"),
        Line::from("Table"),
        Line::from("Dependencies"),
    ])
    .select(app.view.index())
    .highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(tabs, area);
}

fn draw_board(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.view {
        View::Kanban => draw_kanban(frame, area, app),
        View::List => draw_list(frame, area, app),
        View::Table => draw_table(frame, area, app),
        View::Graph => draw_graph(frame, area, app),
    }
}

fn draw_kanban(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let columns = app.columns();
    let statuses = if area.width >= 88 {
        columns.clone()
    } else {
        vec![app.active_status()]
    };
    let constraints = vec![Constraint::Ratio(1, statuses.len() as u32); statuses.len()];
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (pane, status) in panes.iter().zip(statuses.iter()) {
        let tickets = app.visible_for_status(status);
        let active = *status == app.active_status();
        let title = format!("{}  {}", status_label(status), tickets.len());
        let border = if active { Color::Cyan } else { Color::DarkGray };
        let items = tickets
            .iter()
            .map(|ticket| card_item(ticket, pane.width.saturating_sub(4) as usize))
            .collect::<Vec<_>>();
        let selected = if active && !tickets.is_empty() {
            Some(app.selected.min(tickets.len() - 1))
        } else {
            None
        };
        let mut state = ListState::default();
        state.select(selected);
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border))
                        .title(title),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Rgb(39, 48, 68))
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› "),
            *pane,
            &mut state,
        );
    }
}

fn card_item(ticket: &Ticket, max_width: usize) -> ListItem<'static> {
    let summary = truncate(&ticket.title, max_width.max(8));
    let meta = format!("{} · P{} · {}", ticket.id, ticket.priority, ticket.kind);
    ListItem::new(vec![
        Line::from(Span::styled(
            summary,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            meta,
            Style::default().fg(priority_color(ticket.priority)),
        )),
        Line::from(""),
    ])
}

fn draw_list(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let tickets = app.visible_flat();
    let items = tickets
        .iter()
        .map(|ticket| {
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!("{:<10}", ticket.id),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("P{}", ticket.priority),
                        Style::default().fg(priority_color(ticket.priority)),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        status_label(&ticket.status),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(format!("  {}", ticket.title)),
                ]),
                Line::from(Span::styled(
                    format!("            {}  {}", ticket.kind, ticket.assignee),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state
        .select((!tickets.is_empty()).then_some(app.selected.min(tickets.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Tickets  {}", tickets.len())),
            )
            .highlight_style(Style::default().bg(Color::Rgb(39, 48, 68)).fg(Color::White))
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_table(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let tickets = app.visible_flat();
    let rows = tickets
        .iter()
        .enumerate()
        .map(|(index, ticket)| {
            let style = if index == app.selected {
                Style::default().bg(Color::Rgb(39, 48, 68)).fg(Color::White)
            } else {
                Style::default()
            };
            Row::new(vec![
                ticket.id.clone(),
                status_label(&ticket.status),
                format!("P{}", ticket.priority),
                ticket.kind.clone(),
                ticket.title.clone(),
            ])
            .style(style)
        })
        .collect::<Vec<_>>();
    let header = Row::new(vec!["ID", "Status", "Pri", "Type", "Title"]).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(5),
                Constraint::Length(10),
                Constraint::Min(12),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Tickets  {}", tickets.len())),
        ),
        area,
    );
}

#[derive(Clone, Copy)]
struct GraphEdge {
    source: usize,
    target: usize,
}

#[derive(Clone, Copy, Default)]
struct GraphCell {
    ports: u8,
}

const GRAPH_UP: u8 = 1;
const GRAPH_DOWN: u8 = 2;
const GRAPH_LEFT: u8 = 4;
const GRAPH_RIGHT: u8 = 8;
const GRAPH_UP_DOWN: u8 = GRAPH_UP | GRAPH_DOWN;
const GRAPH_LEFT_RIGHT: u8 = GRAPH_LEFT | GRAPH_RIGHT;
const GRAPH_DOWN_RIGHT: u8 = GRAPH_DOWN | GRAPH_RIGHT;
const GRAPH_UP_RIGHT: u8 = GRAPH_UP | GRAPH_RIGHT;
const GRAPH_DOWN_LEFT: u8 = GRAPH_DOWN | GRAPH_LEFT;
const GRAPH_UP_LEFT: u8 = GRAPH_UP | GRAPH_LEFT;
const GRAPH_UP_DOWN_RIGHT: u8 = GRAPH_UP | GRAPH_DOWN | GRAPH_RIGHT;
const GRAPH_UP_DOWN_LEFT: u8 = GRAPH_UP | GRAPH_DOWN | GRAPH_LEFT;
const GRAPH_UP_LEFT_RIGHT: u8 = GRAPH_UP | GRAPH_LEFT | GRAPH_RIGHT;
const GRAPH_DOWN_LEFT_RIGHT: u8 = GRAPH_DOWN | GRAPH_LEFT | GRAPH_RIGHT;
const GRAPH_ALL: u8 = GRAPH_UP | GRAPH_DOWN | GRAPH_LEFT | GRAPH_RIGHT;

fn graph_layout(tickets: &[&Ticket]) -> (Vec<GraphEdge>, Vec<usize>, Vec<usize>) {
    let indexes = tickets
        .iter()
        .enumerate()
        .map(|(index, ticket)| (ticket.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut pairs = Vec::new();
    for (target, ticket) in tickets.iter().enumerate() {
        for dependency in &ticket.dependencies {
            if let Some(source) = indexes.get(dependency.as_str()).copied() {
                // Topological ordering puts prerequisites above their dependents.
                // Backward edges are left in the detail panel rather than drawn
                // in reverse through the rail layout.
                if source < target && !pairs.contains(&(source, target)) {
                    pairs.push((source, target));
                }
            }
        }
    }
    pairs.sort_unstable();

    let mut last_dependents = vec![None; tickets.len()];
    let edges = pairs
        .into_iter()
        .map(|(source, target)| {
            last_dependents[source] =
                Some(last_dependents[source].map_or(target, |last: usize| last.max(target)));
            GraphEdge { source, target }
        })
        .collect::<Vec<_>>();
    let last_dependent_rows = last_dependents
        .iter()
        .enumerate()
        .map(|(row, last)| last.unwrap_or(row))
        .collect::<Vec<_>>();

    // Keep a ticket on its own rail until its final dependent. A later ticket
    // can reuse that rail on the same row where the old path reaches it, which
    // makes ordinary chains compact and lets joins land directly on a node.
    let mut lane_ends = Vec::new();
    let mut node_lanes = Vec::with_capacity(tickets.len());
    for row in 0..last_dependents.len() {
        let lane = lane_ends
            .iter()
            .position(|end| *end <= row)
            .unwrap_or_else(|| {
                lane_ends.push(row);
                lane_ends.len() - 1
            });
        lane_ends[lane] = last_dependent_rows[row];
        node_lanes.push(lane);
    }
    (edges, node_lanes, last_dependent_rows)
}

fn graph_glyph(ports: u8) -> &'static str {
    match ports {
        GRAPH_UP_DOWN => "│",
        GRAPH_LEFT_RIGHT => "─",
        GRAPH_DOWN_RIGHT => "┌",
        GRAPH_UP_RIGHT => "└",
        GRAPH_DOWN_LEFT => "┐",
        GRAPH_UP_LEFT => "┘",
        GRAPH_UP_DOWN_RIGHT => "├",
        GRAPH_UP_DOWN_LEFT => "┤",
        GRAPH_UP_LEFT_RIGHT => "┴",
        GRAPH_DOWN_LEFT_RIGHT => "┬",
        GRAPH_ALL => "┼",
        _ => " ",
    }
}

fn draw_graph(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let tickets = app.visible_graph();
    let tickets_by_id = app
        .tickets
        .iter()
        .map(|ticket| (ticket.id.as_str(), ticket))
        .collect::<HashMap<_, _>>();
    let selected = app.selected_ticket();
    let selected_id = selected.map(|ticket| ticket.id.clone());
    let selected_dependencies = selected
        .map(|ticket| {
            ticket
                .dependencies
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let selected_dependents = selected_id
        .as_deref()
        .map(|id| {
            app.tickets
                .iter()
                .filter(|ticket| {
                    ticket
                        .dependencies
                        .iter()
                        .any(|dependency| dependency == id)
                })
                .map(|ticket| ticket.id.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (edges, node_lanes, last_dependent_rows) = graph_layout(&tickets);
    let lane_count = node_lanes.iter().copied().max().map_or(1, |lane| lane + 1);
    let node_column = lane_count.saturating_sub(1).saturating_mul(2);
    let graph_width = node_column + 1;
    let mut graph_rows = vec![vec![GraphCell::default(); graph_width]; tickets.len()];
    let mut rail_colors = vec![vec![(Color::DarkGray, 0_u8); graph_width]; tickets.len()];
    for edge in &edges {
        let source_column = node_lanes[edge.source] * 2;
        let target_column = node_lanes[edge.target] * 2;
        let edge_color = if selected_id.as_deref() == Some(tickets[edge.source].id.as_str()) {
            Color::Cyan
        } else if selected_id.as_deref() == Some(tickets[edge.target].id.as_str()) {
            Color::Green
        } else {
            Color::DarkGray
        };
        let color_rank = u8::from(edge_color != Color::DarkGray);
        for row_index in edge.source + 1..=edge.target {
            graph_rows[row_index][source_column].ports |= GRAPH_UP;
            if row_index < last_dependent_rows[edge.source] {
                graph_rows[row_index][source_column].ports |= GRAPH_DOWN;
            }
            if row_index == edge.target && source_column != target_column {
                let (start, end, source_port, target_port) = if source_column < target_column {
                    (source_column, target_column, GRAPH_RIGHT, GRAPH_LEFT)
                } else {
                    (target_column, source_column, GRAPH_LEFT, GRAPH_RIGHT)
                };
                graph_rows[row_index][source_column].ports |= source_port;
                for column in start + 1..end {
                    graph_rows[row_index][column].ports |= GRAPH_LEFT | GRAPH_RIGHT;
                }
                graph_rows[row_index][target_column].ports |= target_port;
            }
            let color_end = if row_index == edge.target {
                source_column.max(target_column)
            } else {
                source_column
            };
            let color_start = if row_index == edge.target {
                source_column.min(target_column)
            } else {
                source_column
            };
            for column in color_start..=color_end {
                if color_rank >= rail_colors[row_index][column].1 {
                    rail_colors[row_index][column] = (edge_color, color_rank);
                }
            }
        }
    }

    let content_width = area.width.saturating_sub(4) as usize;
    let items = tickets
        .iter()
        .enumerate()
        .map(|(row_index, ticket)| {
            let state = dependency_state(ticket, &tickets_by_id);
            let state_color = match state {
                "READY" => Color::Green,
                "WAITING" => Color::Yellow,
                _ => Color::Red,
            };
            let node_color = if selected_id.as_deref() == Some(ticket.id.as_str()) {
                Color::Yellow
            } else if selected_dependencies
                .iter()
                .any(|dependency| *dependency == ticket.id)
            {
                Color::Green
            } else if selected_dependents.contains(&ticket.id.as_str()) {
                Color::Cyan
            } else {
                dependency_color(&ticket.status)
            };
            let tail = format!("{} · {state}", status_label(&ticket.status));
            let fixed_width =
                graph_width + 1 + ticket.id.chars().count() + 2 + tail.chars().count() + 2;
            let title_width = content_width.saturating_sub(fixed_width);
            let mut spans = Vec::with_capacity(graph_width + 6);
            for column in 0..graph_width {
                let glyph = if column == node_lanes[row_index] * 2 {
                    "●"
                } else {
                    graph_glyph(graph_rows[row_index][column].ports)
                };
                let color = if column == node_lanes[row_index] * 2 {
                    node_color
                } else {
                    rail_colors[row_index][column].0
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                ticket.id.as_str(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                truncate(&ticket.title, title_width),
                Style::default().fg(Color::White),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(tail, Style::default().fg(state_color)));
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();

    let mut state = ListState::default();
    state
        .select((!tickets.is_empty()).then_some(app.selected.min(tickets.len().saturating_sub(1))));
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(format!(
                "Dependencies  {}  (j/k select · ● ticket · rails show prerequisites)",
                tickets.len()
            )))
            .highlight_style(Style::default().bg(Color::Rgb(39, 48, 68)).fg(Color::White))
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_detail(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(ticket) = app.selected_ticket() else {
        frame.render_widget(
            Paragraph::new("Select a ticket to see details")
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).title("Details")),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::from(Span::styled(
            &ticket.title,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("ID       ", Style::default().fg(Color::DarkGray)),
            Span::raw(ticket.id.as_str()),
        ]),
        Line::from(vec![
            Span::styled("Status   ", Style::default().fg(Color::DarkGray)),
            Span::raw(status_label(&ticket.status)),
        ]),
        Line::from(vec![
            Span::styled("Priority ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("P{}", ticket.priority)),
        ]),
        Line::from(vec![
            Span::styled("Type     ", Style::default().fg(Color::DarkGray)),
            Span::raw(ticket.kind.as_str()),
        ]),
    ];
    if !ticket.assignee.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Assignee ", Style::default().fg(Color::DarkGray)),
            Span::raw(ticket.assignee.as_str()),
        ]));
    }
    if !ticket.tags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Tags     ", Style::default().fg(Color::DarkGray)),
            Span::raw(ticket.tags.join(", ")),
        ]));
    }
    if !ticket.dependencies.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Depends  ", Style::default().fg(Color::DarkGray)),
            Span::raw(ticket.dependencies.join(", ")),
        ]));
    }
    let unlocks = app
        .tickets
        .iter()
        .filter(|candidate| {
            candidate
                .dependencies
                .iter()
                .any(|dependency| dependency == &ticket.id)
        })
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>();
    if !unlocks.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Unlocks  ", Style::default().fg(Color::DarkGray)),
            Span::raw(unlocks.join(", ")),
        ]));
    }
    if !ticket.description.is_empty() {
        lines.push(Line::from(""));
        lines.extend(ticket.description.lines().map(Line::from));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("Details")),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let hints = if app.moving {
        "MOVE  h/l move  Esc cancel"
    } else {
        "K/Tab view  j/k select  v move  a add  x close  o reopen  / filter  C closed  r reload  ? help  q/Esc quit"
    };
    let info = if app.message.is_empty() {
        hints.to_owned()
    } else {
        format!("{}    {}", app.message, hints)
    };
    frame.render_widget(
        Paragraph::new(truncate(&info, area.width as usize)).style(Style::default().fg(
            if app.message.contains("failed") || app.message.contains("Could not") {
                Color::Red
            } else {
                Color::DarkGray
            },
        )),
        area,
    );
}

fn draw_create_dialog(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(70, 40, frame.area());
    frame.render_widget(Clear, area);
    let title_style = if app.create.field == 0 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let description_style = if app.create.field == 1 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let lines = vec![
        Line::from(Span::styled(
            "New ticket",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("Title", title_style)),
        Line::from(format!("> {}", app.create.title)),
        Line::from(""),
        Line::from(Span::styled("Description (optional)", description_style)),
        Line::from(format!("> {}", app.create.description)),
        Line::from(""),
        Line::from(Span::styled(
            "Tab switches fields · Enter advances/creates · Esc cancels",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("Create"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_filter_dialog(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(65, 20, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Filter by id, title, status, type, assignee, or tag"),
            Line::from(""),
            Line::from(format!("/ {}", app.input)),
            Line::from(""),
            Line::from(Span::styled(
                "Enter apply · Esc clear",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("Filter")),
        area,
    );
}

fn draw_help(frame: &mut Frame<'_>) {
    let area = centered_rect(72, 76, frame.area());
    frame.render_widget(Clear, area);
    let help = vec![
        Line::from(Span::styled(
            "herdr-ticket keys",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("K / Tab       cycle Kanban, List, Table, Dependencies"),
        Line::from("j / k         move selection"),
        Line::from("h / l         change kanban column"),
        Line::from("v then h/l    move selected ticket to another status"),
        Line::from("a             create a ticket with tk"),
        Line::from("x / o         close / reopen ticket"),
        Line::from("s             cycle open → in progress → closed"),
        Line::from("Enter / d     toggle ticket details"),
        Line::from("/             filter by text"),
        Line::from("C             show or hide closed tickets"),
        Line::from("Dependencies  branch rails connect prerequisites and joins"),
        Line::from("r             reload from .tickets/ (also watched automatically)"),
        Line::from("q / Esc       close the board"),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to return",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(help))
            .block(Block::default().borders(Borders::ALL).title("Help")),
        area,
    );
}

fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

fn truncate(text: &str, max_width: usize) -> String {
    let width = text.chars().count();
    if width <= max_width {
        return text.to_owned();
    }
    if max_width <= 1 {
        return "…".to_owned();
    }
    text.chars().take(max_width - 1).collect::<String>() + "…"
}

fn priority_color(priority: u8) -> Color {
    match priority {
        0 => Color::Red,
        1 => Color::LightRed,
        2 => Color::Yellow,
        3 => Color::Green,
        _ => Color::DarkGray,
    }
}
