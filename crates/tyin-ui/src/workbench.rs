use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tyin_fs::FsEntry;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Files,
    Search,
    SourceControl,
    Room,
    Extensions,
}

#[derive(Clone, Debug)]
pub(super) struct SearchHit {
    pub path: PathBuf,
    pub line: usize,
    pub preview: String,
}

enum SearchEvent {
    Finished(u64, Vec<SearchHit>),
}

pub(super) struct SearchState {
    pub query: String,
    pub case_sensitive: bool,
    pub matches: Vec<SearchHit>,
    searching: bool,
    request_id: u64,
    generation: Arc<AtomicU64>,
    sender: Sender<SearchEvent>,
    receiver: Receiver<SearchEvent>,
}

fn matches_in_file(
    path: &std::path::Path,
    text: &str,
    needle: &str,
    case_sensitive: bool,
    limit: usize,
) -> Vec<SearchHit> {
    text.lines()
        .enumerate()
        .filter_map(|(line_index, line)| {
            let haystack = if case_sensitive {
                line.to_owned()
            } else {
                line.to_lowercase()
            };
            haystack.contains(needle).then(|| SearchHit {
                path: path.to_path_buf(),
                line: line_index + 1,
                preview: line.trim().chars().take(180).collect(),
            })
        })
        .take(limit)
        .collect()
}

impl SearchState {
    pub(super) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            query: String::new(),
            case_sensitive: false,
            matches: Vec::new(),
            searching: false,
            request_id: 0,
            generation: Arc::new(AtomicU64::new(0)),
            sender,
            receiver,
        }
    }

    fn start(&mut self, entries: Vec<FsEntry>) {
        self.request_id = self.request_id.wrapping_add(1);
        let request_id = self.request_id;
        self.generation.store(request_id, Ordering::Relaxed);
        self.searching = true;
        self.matches.clear();
        let query = self.query.clone();
        let case_sensitive = self.case_sensitive;
        let sender = self.sender.clone();
        let generation = self.generation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            if generation.load(Ordering::Relaxed) != request_id {
                return;
            }
            let needle = if case_sensitive {
                query
            } else {
                query.to_lowercase()
            };
            let mut hits = Vec::new();
            'files: for entry in entries.into_iter().filter(|entry| !entry.is_dir) {
                if generation.load(Ordering::Relaxed) != request_id {
                    return;
                }
                let Ok(metadata) = std::fs::metadata(&entry.path) else {
                    continue;
                };
                if metadata.len() > 1024 * 1024 {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&entry.path) else {
                    continue;
                };
                let file_hits = matches_in_file(
                    &entry.path,
                    &text,
                    &needle,
                    case_sensitive,
                    500 - hits.len(),
                );
                hits.extend(file_hits);
                if hits.len() == 500 {
                    break 'files;
                }
            }
            let _ = sender.send(SearchEvent::Finished(request_id, hits));
        });
    }

    pub(super) fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                SearchEvent::Finished(request_id, matches) if request_id == self.request_id => {
                    self.matches = matches;
                    self.searching = false;
                }
                SearchEvent::Finished(_, _) => {}
            }
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        root: Option<&std::path::Path>,
        entries: &[FsEntry],
    ) -> Option<SearchHit> {
        self.poll();
        section_header(ui, "SEARCH");
        let mut open_hit = None;
        ui.add_space(8.0);
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Search workspace")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            if self.query.trim().len() >= 2 {
                if root.is_some() {
                    self.start(entries.to_vec());
                }
            } else {
                self.request_id = self.request_id.wrapping_add(1);
                self.generation.store(self.request_id, Ordering::Relaxed);
                self.searching = false;
                self.matches.clear();
            }
        }
        let mut case_changed = false;
        ui.horizontal(|ui| {
            case_changed = ui.checkbox(&mut self.case_sensitive, "Aa").changed();
            ui.weak("Match case");
            if ui.button("Clear").clicked() {
                self.query.clear();
                self.matches.clear();
                self.searching = false;
                self.request_id = self.request_id.wrapping_add(1);
                self.generation.store(self.request_id, Ordering::Relaxed);
            }
        });
        if case_changed && self.query.trim().len() >= 2 && root.is_some() {
            self.start(entries.to_vec());
        }
        ui.separator();

        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Searching workspace…");
            });
        } else if self.query.trim().len() < 2 {
            ui.label("Type at least two characters to search.");
        } else if self.matches.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.strong("No results");
                ui.weak("Try a different query or search a larger workspace.");
                if ui.button("Clear search").clicked() {
                    self.query.clear();
                }
            });
        } else {
            let mut grouped: Vec<(PathBuf, Vec<SearchHit>)> = Vec::new();
            for hit in &self.matches {
                if let Some((_, matches)) = grouped.iter_mut().find(|(path, _)| path == &hit.path) {
                    matches.push(hit.clone());
                } else {
                    grouped.push((hit.path.clone(), vec![hit.clone()]));
                }
            }
            ui.label(format!("{} RESULTS", self.matches.len()));
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for (path, matches) in grouped {
                        ui.add_space(6.0);
                        let relative = root
                            .and_then(|root| path.strip_prefix(root).ok())
                            .unwrap_or(path.as_path())
                            .display()
                            .to_string();
                        egui::CollapsingHeader::new(format!("{}  ·  {}", relative, matches.len()))
                            .default_open(true)
                            .show(ui, |ui| {
                                for hit in matches {
                                    let text = egui::RichText::new(format!(
                                        "{:>4}  {}",
                                        hit.line, hit.preview
                                    ))
                                    .monospace()
                                    .size(10.0);
                                    if ui
                                        .add_sized(
                                            [ui.available_width(), 24.0],
                                            egui::Button::new(text).frame(false),
                                        )
                                        .clicked()
                                    {
                                        open_hit = Some(hit);
                                    }
                                }
                            });
                    }
                });
        }
        open_hit
    }
}

#[derive(Clone)]
pub(super) struct RoomMessage {
    author: &'static str,
    initials: &'static str,
    time: &'static str,
    text: String,
}

pub(super) fn initial_room_messages() -> Vec<RoomMessage> {
    vec![
        RoomMessage {
            author: "Ari",
            initials: "AR",
            time: "10:24",
            text: "The reconnect patch is ready for review. I kept the session alive while the tab sleeps.".into(),
        },
        RoomMessage {
            author: "Jules",
            initials: "JD",
            time: "10:26",
            text: "Looks good. Could we add a small backoff before the first retry?".into(),
        },
        RoomMessage {
            author: "Mei",
            initials: "MK",
            time: "10:29",
            text: "Added the backoff and a regression test.".into(),
        },
        RoomMessage {
            author: "Ari",
            initials: "AR",
            time: "10:31",
            text: "Nice. Shipping once CI is green.".into(),
        },
    ]
}

pub(super) fn show_room(
    ui: &mut egui::Ui,
    query: &mut String,
    draft: &mut String,
    messages: &mut Vec<RoomMessage>,
    focus_search: bool,
) {
    ui.horizontal(|ui| {
        let mark = ui.add_sized([26.0, 26.0], egui::Button::new("#"));
        let _ = mark;
        ui.vertical(|ui| {
            ui.strong("Design system");
            ui.weak("4 online · 12 members");
        });
    });
    ui.add_space(8.0);
    let response = ui.add(
        egui::TextEdit::singleline(query)
            .id(egui::Id::new("room_search"))
            .hint_text("Search in room  ·  Ctrl+F"),
    );
    if focus_search {
        response.request_focus();
    }
    ui.add_space(8.0);
    ui.label("PINNED  ·  RFC · session lifecycle");
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for message in messages.iter().filter(|message| {
                query.is_empty() || message.text.to_lowercase().contains(&query.to_lowercase())
            }) {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [26.0, 26.0],
                        egui::Button::new(egui::RichText::new(message.initials).small().strong()),
                    );
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(message.author);
                            ui.weak(egui::RichText::new(message.time).monospace());
                        });
                        ui.label(&message.text);
                    });
                });
                ui.add_space(10.0);
            }
            ui.weak("··· Mei is typing");
            ui.add_space(6.0);
            ui.weak("Local preview — messages are not synced.");
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(draft)
                .hint_text("Message #design-system")
                .desired_width(f32::INFINITY),
        );
        let send = ui.button("↑").clicked()
            || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if send && !draft.trim().is_empty() {
            messages.push(RoomMessage {
                author: "You",
                initials: "TY",
                time: "now",
                text: std::mem::take(draft),
            });
        }
    });
}

pub(super) fn show_assistant(ui: &mut egui::Ui, draft: &mut String) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [28.0, 28.0],
            egui::Button::new(egui::RichText::new("✦").strong()),
        );
        ui.vertical(|ui| {
            ui.strong("Tyin Assistant");
            ui.weak("AI workspace");
        });
    });
    ui.separator();
    ui.weak("AI provider is not configured");
    ui.add_space(10.0);
    ui.label(
        "Connect an AI provider to ask questions about your workspace and review proposed changes.",
    );
    ui.add_space(10.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.weak("Available context");
        ui.label("Open files and workspace");
    });
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            let _ = ui.add_enabled(
                false,
                egui::TextEdit::singleline(draft).hint_text("Ask about your code…"),
            );
            let _ = ui.add_enabled(false, egui::Button::new("↑"));
        });
    });
}

pub(super) fn show_extensions(
    ui: &mut egui::Ui,
    query: &mut String,
    marketplace: &mut bool,
    selected: &mut String,
) {
    section_header(ui, "EXTENSIONS");
    ui.add_space(8.0);
    ui.add(egui::TextEdit::singleline(query).hint_text("Search extensions"));
    ui.horizontal(|ui| {
        if ui.selectable_label(!*marketplace, "Installed  4").clicked() {
            *marketplace = false;
        }
        if ui.selectable_label(*marketplace, "Marketplace").clicked() {
            *marketplace = true;
        }
    });
    ui.separator();
    let rows = [
        ("nova-lint", "Nova Lint", "nova-labs · 1.2M installs", true),
        ("graph-lens", "Graph Lens", "acme · 840k installs", true),
        (
            "mono-theme",
            "Mono Theme",
            "nova-labs · 2.4M installs",
            true,
        ),
        ("test-runner", "Test Runner", "vitest · 3.1M installs", true),
        (
            "pair-session",
            "Pair Session",
            "realtime · 120k installs",
            false,
        ),
        (
            "rest-client",
            "REST Client",
            "httpie · 4.2M installs",
            false,
        ),
    ];
    let query = query.to_lowercase();
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for (id, name, subtitle, installed) in rows {
                if !query.is_empty()
                    && !name.to_lowercase().contains(&query)
                    && !subtitle.to_lowercase().contains(&query)
                {
                    continue;
                }
                if !*marketplace && !installed {
                    continue;
                }
                let is_selected = *selected == id;
                let label = if is_selected {
                    format!("◆  {name}\n    {subtitle}")
                } else {
                    format!("◇  {name}\n    {subtitle}")
                };
                if ui
                    .add_sized(
                        [ui.available_width(), 48.0],
                        egui::SelectableLabel::new(is_selected, label),
                    )
                    .clicked()
                {
                    *selected = id.to_owned();
                }
            }
        });
    ui.separator();
    ui.weak("Marketplace preview · installation host not configured");
}

pub(super) fn show_extension_detail(ui: &mut egui::Ui, selected: &str) {
    let (name, publisher, version, description, installed, license, size) = match selected {
        "graph-lens" => (
            "Graph Lens",
            "@acme",
            "3.2.0",
            "Explore module dependencies and trace imports across your workspace.",
            false,
            "Apache-2.0",
            "8.2 MB",
        ),
        "mono-theme" => (
            "Mono Theme",
            "@nova-labs",
            "1.8.0",
            "A quiet monochrome theme for focused editing and code review.",
            true,
            "MIT",
            "1.4 MB",
        ),
        "test-runner" => (
            "Test Runner",
            "@vitest",
            "2.1.0",
            "Run and inspect tests from the workspace without leaving the editor.",
            true,
            "MIT",
            "6.8 MB",
        ),
        "pair-session" => (
            "Pair Session",
            "@realtime",
            "0.9.4",
            "Collaborate on files and review changes together in real time.",
            false,
            "MIT",
            "4.1 MB",
        ),
        "rest-client" => (
            "REST Client",
            "@httpie",
            "1.4.2",
            "Send HTTP requests from a workspace collection.",
            false,
            "MIT",
            "3.6 MB",
        ),
        _ => (
            "Nova Lint",
            "@nova-labs",
            "2.4.1",
            "Workspace linting with inline diagnostics, quiet mode, and automatic fixes.",
            true,
            "MIT",
            "12.4 MB",
        ),
    };

    ui.horizontal(|ui| {
        ui.label("marketplace  /  extensions  /");
        ui.monospace(selected);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(format!("v{version}"));
        });
    });
    ui.separator();
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.add_sized([64.0, 64.0], egui::Button::new("◇"));
        ui.vertical(|ui| {
            ui.heading(name);
            ui.label(format!("by {publisher} · ★ 4.8 · verified"));
            ui.horizontal(|ui| {
                for tag in ["Developer tools", "Workspace", "Productivity"] {
                    let _ = ui.small_button(tag);
                }
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.add_enabled(
                false,
                egui::Button::new(if installed {
                    "Installed"
                } else {
                    "Install unavailable"
                }),
            )
            .on_hover_text("Extension installation is not available in this build.");
        });
    });
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.columns(2, |columns| {
            columns[0].heading("Overview");
            columns[0].label(description);
            columns[0].add_space(12.0);
            columns[0].heading("Features");
            for feature in [
                "Workspace-aware and designed for large projects",
                "Keyboard-first navigation",
                "Settings scoped to the current workspace",
            ] {
                columns[0].label(format!("✓  {feature}"));
            }
            columns[0].add_space(14.0);
            columns[0].heading("Changelog");
            columns[0].label(format!("{version}  ·  Current stable release"));
            columns[0].label("Improved startup and workspace reliability");

            columns[1].heading("Details");
            egui::Grid::new("extension_details")
                .num_columns(2)
                .show(&mut columns[1], |ui| {
                    for (label, value) in [
                        ("Version", version),
                        ("Publisher", publisher),
                        ("License", license),
                        ("Size", size),
                        ("Engine", "Tyin ≥ 0.1"),
                    ] {
                        ui.weak(label);
                        ui.monospace(value);
                        ui.end_row();
                    }
                });
            columns[1].add_space(14.0);
            columns[1].heading("Permissions");
            columns[1].label("Read workspace files");
            columns[1].label("Network access, when required");
            columns[1].add_space(12.0);
            columns[1].weak("Preview only. No extension is installed or loaded.");
        });
    });
}

pub(super) fn show_diff(ui: &mut egui::Ui, diff: Option<&str>, selected: Option<&std::path::Path>) {
    ui.horizontal(|ui| {
        ui.strong("WORKING TREE");
        ui.weak("↔");
        ui.monospace(
            selected
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Select a changed file".into()),
        );
    });
    ui.separator();
    let Some(diff) = diff else {
        ui.centered_and_justified(|ui| {
            ui.weak("Select a changed file in Source control to review its diff.");
        });
        return;
    };
    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for line in diff.lines() {
                let text = if line.starts_with('+') && !line.starts_with("+++") {
                    egui::RichText::new(line).color(egui::Color32::from_rgb(143, 183, 158))
                } else if line.starts_with('-') && !line.starts_with("---") {
                    egui::RichText::new(line).color(egui::Color32::from_rgb(190, 139, 139))
                } else {
                    egui::RichText::new(line).weak()
                };
                ui.label(text.monospace().size(11.0));
            }
        });
}

fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).small().strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = ui.small_button("···");
        });
    });
}

#[cfg(test)]
mod tests {
    use super::matches_in_file;
    use std::path::Path;

    #[test]
    fn workspace_search_respects_case_and_caps_line_matches() {
        let text = "Store = 1\nstore = 2\nSTORE = 3";
        let insensitive = matches_in_file(Path::new("src/store.rs"), text, "store", false, 2);
        assert_eq!(insensitive.len(), 2);
        assert_eq!(insensitive[1].line, 2);

        let sensitive = matches_in_file(Path::new("src/store.rs"), text, "Store", true, 10);
        assert_eq!(sensitive.len(), 1);
        assert_eq!(sensitive[0].line, 1);
    }
}
