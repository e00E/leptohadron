// todos:
// - handle %PROVIDES%, for example mailcap provides mime-types
// - better error handling
// - figure out reasonable way to do logging, maybe print after main ends or detect whether stderr is tty

// ideas:
// - mode where left side shows only explicit installed and is recursive so you get a list of all packages
//   you'd have to remove

mod installed_packages;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use crossterm::{
    event::{Event, KeyCode},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use installed_packages::{PackageDesc, Reason};
use tui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Row, Table, Wrap},
    Frame, Terminal,
};

const HELP: &[(&str, &str)] = &[
    ("left, right", "move between lists"),
    ("up, down, PgUp, PgDown", "move in list"),
    ("1, 0", "move to start/end of list"),
    ("Enter", "focus center list on selected entry"),
    (
        "s",
        "toggle sorting between alphabetical-asc and size-desc in active view",
    ),
    (
        "e",
        "toggle showing only explicitly installed packages in main view",
    ),
    (
        "/",
        "start entering search term, enter to search, esc to cancel",
    ),
    ("n", "go to next search match downwards"),
    ("N", "go to next search match upwards"),
    ("?", "toggle help"),
    ("q", "quit"),
];

#[derive(Clone, Copy, Default)]
enum SortCritera {
    #[default]
    NameAsc,
    SizeDesc,
}

impl SortCritera {
    fn sort(&self, packages: &mut [&PackageDesc]) {
        match self {
            SortCritera::NameAsc => packages.sort_by_key(|package| package.name.as_str()),
            SortCritera::SizeDesc => {
                packages.sort_by_key(|package| std::cmp::Reverse(package.size.unwrap_or(0)))
            }
        };
    }
}

#[derive(Clone, Copy, Default)]
enum Filter {
    #[default]
    All,
    Explicit,
}

impl Filter {
    fn filter(&self, package: &PackageDesc) -> bool {
        match self {
            Self::All => true,
            Self::Explicit => matches!(package.reason, Reason::Explicit),
        }
    }
}

#[derive(Default)]
struct Column<'a> {
    title: &'static str,
    is_active: bool,
    sort_criteria: SortCritera,
    packages: Vec<&'a PackageDesc>,
    // invariant: never has element selected that is out of range of `packages`
    // invariant: has no selection IFF packages is empty
    list_state: ListState,
}

impl<'a> Column<'a> {
    fn render(&mut self, frame: &mut Frame<impl Backend>, area: Rect) {
        let block = Block::default()
            .title(format!(
                "{} {}/{}",
                self.title,
                self.list_state.selected().map(|i| i + 1).unwrap_or(0),
                self.packages.len()
            ))
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_type(match self.is_active {
                true => BorderType::Thick,
                false => BorderType::Plain,
            });
        let area_ = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 2); 2])
            .split(area_);

        let items: Vec<ListItem> = self
            .packages
            .iter()
            .map(|desc| ListItem::new(Text::raw(desc.name.as_str())))
            .collect();
        let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::BOLD));
        frame.render_stateful_widget(list, chunks[0], &mut self.list_state);

        let selected = self.selected();
        let mut text: Vec<Spans> = Default::default();
        if let Some(selected) = selected {
            let style = Style::default().add_modifier(Modifier::UNDERLINED);
            text.push(Spans(vec![
                Span::styled("name", style),
                format!(":    {}", selected.name).into(),
            ]));
            text.push(Spans(vec![
                Span::styled("version", style),
                format!(": {}", selected.version).into(),
            ]));
            text.push(Spans(vec![
                Span::styled("reason", style),
                format!(":  {:?}", selected.reason).into(),
            ]));
            text.push(Spans(vec![
                Span::styled("size", style),
                format!(
                    ":    {}",
                    humansize::SizeFormatter::new(selected.size.unwrap_or(0), humansize::DECIMAL)
                )
                .into(),
            ]));
            text.push("".into());
            text.push(Spans(vec![Span::styled("description", style), ":".into()]));
            text.push(selected.description.as_str().into());
            text.push("".into());
            text.push(Spans(vec![Span::styled("url", style), ":".into()]));
            text.push(selected.url.as_str().into());
        }
        let paragraph = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::TOP));
        frame.render_widget(paragraph, chunks[1]);
    }

    fn change(&mut self, distance: isize) -> bool {
        let old = self.list_state.selected();
        let new = old
            .map(|i| (i as isize + distance).clamp(0, (self.packages.len() - 1) as isize) as usize);
        self.list_state.select(new);
        old != new
    }

    fn after_packages_change(&mut self, previous_selection: Option<&'a PackageDesc>) {
        let new_index = previous_selection.and_then(|package| {
            self.packages
                .iter()
                .position(|package_| std::ptr::eq(package, *package_))
        });
        self.list_state
            .select(new_index.or_else(|| (!self.packages.is_empty()).then_some(0)));
    }

    fn selected(&self) -> Option<&'a PackageDesc> {
        self.list_state
            .selected()
            .map(|i| *self.packages.get(i).unwrap())
    }
}

enum SearchDirection {
    Up,
    Down,
}

struct App<'a> {
    packages: &'a BTreeMap<String, PackageDesc>,
    dependants: BTreeMap<&'a str, BTreeSet<&'a str>>,
    columns: [Column<'a>; 3],
    active_column: usize,
    show_help: bool,
    filter: Filter,
    // user is currently entering the search term
    searching: bool,
    // active search term
    search: String,
}

impl<'a> App<'a> {
    fn new(packages: &'a BTreeMap<String, PackageDesc>) -> Self {
        let mut dependants: BTreeMap<&str, BTreeSet<&str>> = Default::default();
        for (name, package) in packages.iter() {
            for dep in package.dependencies.iter().map(|dep| dep.as_str()).chain(
                package
                    .optional_dependencies
                    .iter()
                    .map(|dep| dep.name.as_str()),
            ) {
                // don't insert dependencies that aren't installed
                if packages.contains_key(dep) {
                    dependants.entry(dep).or_default().insert(name.as_str());
                }
            }
        }
        let left = Column {
            title: "Dependants",
            ..Default::default()
        };
        let right = Column {
            title: "Dependencies",
            ..Default::default()
        };
        let mut center = Column {
            title: "All",
            is_active: true,
            sort_criteria: SortCritera::NameAsc,
            packages: packages.values().collect(),
            list_state: Default::default(),
        };
        center.sort_criteria.sort(center.packages.as_mut_slice());
        center.after_packages_change(None);
        let mut self_ = Self {
            packages,
            dependants,
            columns: [left, center, right],
            active_column: 1,
            show_help: true,
            filter: Default::default(),
            searching: false,
            search: String::new(),
        };
        self_.apply_center_filter(Filter::Explicit);
        self_.update_sides(self_.columns[1].selected());
        self_
    }

    fn draw_help(&self, frame: &mut Frame<impl Backend>, area: Rect) {
        let first_row_len = HELP.iter().map(|row| row.0.len()).max().unwrap();
        let constraints = &[
            Constraint::Length(first_row_len as u16),
            Constraint::Ratio(1, 1),
        ];
        let help = Table::new(HELP.iter().map(|row| Row::new(vec![row.0, row.1])))
            .block(
                Block::default()
                    .title("Help")
                    .title_alignment(Alignment::Center)
                    .borders(Borders::ALL),
            )
            .header(Row::new(vec!["Key", "Action"]).bottom_margin(1))
            .widths(constraints);
        frame.render_widget(help, area);
    }

    fn draw_search(&self, frame: &mut Frame<impl Backend>, area: Rect) {
        let text = format!("/{}", self.search);
        let paragraph = Paragraph::new(text);
        frame.render_widget(paragraph, area);
    }

    fn draw(&mut self, frame: &mut Frame<impl Backend>) {
        let area = if self.show_help {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length((HELP.len() + 4) as u16),
                    Constraint::Min(0),
                ])
                .split(frame.size());
            self.draw_help(frame, chunks[0]);
            chunks[1]
        } else {
            frame.size()
        };

        let area = if self.searching {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            self.draw_search(frame, chunks[1]);
            chunks[0]
        } else {
            area
        };

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 3); 3])
            .split(area);

        for (column, chunk) in self.columns.iter_mut().zip(chunks) {
            column.render(frame, chunk);
        }
    }

    // returns whether should quit
    fn event(&mut self, event: Event) -> bool {
        let key = match event {
            Event::Key(key) => key,
            _ => return false,
        };
        let mut list_selection_change = false;
        match key.code {
            KeyCode::Char(char) if self.searching => self.search.push(char),
            KeyCode::Backspace if self.searching => {
                self.search.pop();
            }
            KeyCode::Char('/') => {
                self.searching = true;
                self.search.clear();
            }
            KeyCode::Esc if self.searching => {
                self.searching = false;
                self.search.clear();
            }
            KeyCode::Enter if self.searching => {
                self.searching = false;
                self.active_column = 1;
                list_selection_change = self.search(SearchDirection::Down);
            }
            KeyCode::Char('n') => {
                self.active_column = 1;
                list_selection_change = self.search(SearchDirection::Down);
            }
            KeyCode::Char('N') => {
                self.active_column = 1;
                list_selection_change = self.search(SearchDirection::Up)
            }

            KeyCode::Char('q' | 'c') => return true,

            KeyCode::Left => self.change_active_column(self.active_column.saturating_sub(1)),
            KeyCode::Right => self.change_active_column((self.active_column + 1).min(2)),
            KeyCode::Up => {
                list_selection_change = self.columns[self.active_column].change(-1);
            }
            KeyCode::PageUp => {
                list_selection_change = self.columns[self.active_column].change(-10);
            }
            KeyCode::Down => {
                list_selection_change = self.columns[self.active_column].change(1);
            }
            KeyCode::PageDown => {
                list_selection_change = self.columns[self.active_column].change(10);
            }

            KeyCode::Char('1') => {
                let c = self.columns.get_mut(self.active_column).unwrap();
                if !c.packages.is_empty() {
                    let old = c.list_state.selected().unwrap();
                    let new = 0;
                    c.list_state.select(Some(new));
                    list_selection_change = self.active_column == 1 && old != new;
                }
            }
            KeyCode::Char('0') => {
                let c = self.columns.get_mut(self.active_column).unwrap();
                if !c.packages.is_empty() {
                    let old = c.list_state.selected().unwrap();
                    let new = c.packages.len() - 1;
                    c.list_state.select(Some(new));
                    list_selection_change = self.active_column == 1 && old != new;
                }
            }

            KeyCode::Enter if self.active_column != 1 => {
                self.change_center_package();
            }

            KeyCode::Char('s') => {
                let c = &mut self.columns[self.active_column];
                let selected = c.selected();
                c.sort_criteria = match c.sort_criteria {
                    SortCritera::NameAsc => SortCritera::SizeDesc,
                    SortCritera::SizeDesc => SortCritera::NameAsc,
                };
                c.sort_criteria.sort(c.packages.as_mut_slice());
                c.after_packages_change(selected);
            }

            KeyCode::Char('e') => {
                let filter = match self.filter {
                    Filter::All => Filter::Explicit,
                    Filter::Explicit => Filter::All,
                };
                self.apply_center_filter(filter);
            }

            KeyCode::Char('?') => self.show_help = !self.show_help,

            _ => (),
        }
        if list_selection_change && self.active_column == 1 {
            let package = self.columns.get(1).unwrap().selected();
            self.update_sides(package);
        }
        false
    }

    fn change_active_column(&mut self, new: usize) {
        self.columns.get_mut(self.active_column).unwrap().is_active = false;
        self.columns.get_mut(new).unwrap().is_active = true;
        self.active_column = new;
    }

    fn apply_center_filter(&mut self, filter: Filter) {
        self.filter = filter;
        let c = self.columns.get_mut(1).unwrap();
        let selected = c.selected();
        c.packages = self
            .packages
            .values()
            .filter(|package| self.filter.filter(package))
            .collect();
        c.after_packages_change(selected);
        if let Some(selected) = selected {
            let pos = c
                .packages
                .iter()
                .position(|desc| desc.name == selected.name);
            if let Some(pos) = pos {
                c.list_state.select(Some(pos));
            }
        }
    }

    fn change_center_package(&mut self) {
        let package = match self.columns.get(self.active_column).unwrap().selected() {
            Some(package) => package,
            None => return,
        };
        if let (Reason::Dependency, Filter::Explicit) = (package.reason, self.filter) {
            self.apply_center_filter(Filter::All);
        }
        let c = self.columns.get_mut(1).unwrap();
        c.after_packages_change(Some(package));
        self.update_sides(Some(package));
    }

    fn update_sides(&mut self, package: Option<&PackageDesc>) {
        let package = match package {
            Some(package) => package,
            None => {
                for column in [0, 2] {
                    let c = self.columns.get_mut(column).unwrap();
                    c.packages.clear();
                    c.after_packages_change(None);
                }
                return;
            }
        };
        for (column, mut packages) in [
            (
                0,
                self.dependants
                    .get(package.name.as_str())
                    .into_iter()
                    .flatten()
                    .filter_map(|s| self.packages.get(*s))
                    .collect::<Vec<_>>(),
            ),
            (
                2,
                self.packages
                    .get(package.name.as_str())
                    .into_iter()
                    .flat_map(|desc| {
                        desc.dependencies.iter().map(|s| s.as_str()).chain(
                            desc.optional_dependencies
                                .iter()
                                .map(|dep| dep.name.as_str()),
                        )
                    })
                    .filter_map(|s| self.packages.get(s))
                    .collect::<Vec<_>>(),
            ),
        ] {
            let c = self.columns.get_mut(column).unwrap();
            c.sort_criteria.sort(packages.as_mut_slice());
            c.packages = packages;
            c.after_packages_change(None);
        }
    }

    // returns whether selection changed
    fn search(&mut self, search_direction: SearchDirection) -> bool {
        if self.search.is_empty() {
            return false;
        }
        let c = self.columns.get_mut(1).unwrap();
        let index = match c.list_state.selected() {
            Some(i) => i,
            None => return false,
        };
        let before = c.packages.iter().enumerate().take(index);
        let after = c.packages.iter().enumerate().skip(index + 1);
        let mut iter = after.chain(before);
        let condition =
            |(_, package): &(_, &&PackageDesc)| package.name.contains(self.search.as_str());
        let result = match search_direction {
            SearchDirection::Down => iter.find(condition),
            SearchDirection::Up => iter.rev().find(condition),
        };
        match result {
            Some((index, _)) => {
                c.list_state.select(Some(index));
                true
            }
            None => false,
        }
    }
}

fn main() -> Result<()> {
    const PATH: &str = "/var/lib/pacman/local";
    let packages: BTreeMap<String, PackageDesc> = installed_packages::from_directory(PATH)
        .with_context(|| format!("failed to load installed packages from {PATH}"))?
        .map(|desc| desc.map(|desc| (desc.name.clone(), desc)))
        .collect::<Result<_>>()?;
    let mut app = App::new(&packages);

    let mut stdout = std::io::stdout();
    crossterm::terminal::enable_raw_mode().context("enable_raw_mode")?;
    crossterm::execute!(stdout, EnterAlternateScreen).context("EnterAlternateScreen")?;
    let backend = tui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Terminal::new")?;

    let result = loop {
        match terminal.draw(|frame| app.draw(frame)) {
            Ok(_) => (),
            Err(err) => break Err(err).context("draw"),
        }
        let event = match crossterm::event::read() {
            Ok(event) => event,
            Err(err) => break Err(err).context("crossterm::event::read"),
        };
        if app.event(event) {
            break Ok(());
        }
    };

    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("LeaveAlternateScreen")?;
    crossterm::terminal::disable_raw_mode().context("disable_raw_mode")?;
    terminal.show_cursor().context("show_cursor")?;

    result
}
