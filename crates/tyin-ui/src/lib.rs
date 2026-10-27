use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc::Receiver, Arc};
use std::time::{Duration, Instant};
use tyin_core::{Config, Event, EventBus, ResourceManager};
use tyin_fs::{watch_path, FsEntry, Project, WatcherHandle};
use tyin_git::GitStatus;
use tyin_lsp::{LspCompletion, LspPool};
use tyin_syntax::HighlightSpan;
use tyin_terminal::TerminalSession;
use tyin_text::Buffer;

mod activity_bar;
mod explorer;
mod panel;
mod syntax_view;
mod workspace;

const EDITOR_EXTRA_SCROLL_LINES: usize = 8;

#[derive(Clone, Debug)]
struct Tab {
    id: u64,
    buffer: Buffer,
    path: Option<PathBuf>,
    title: String,
    last_active: Instant,
    cached: bool,
    edit_text: String,
    highlight_spans: Vec<HighlightSpan>,
    syntax_info: String,
    pending_cursor: Option<usize>,
}

fn replace_char_range(text: &str, start: usize, end: usize, insertion: &str) -> (String, usize) {
    let byte_index = |char_index| {
        text.char_indices()
            .nth(char_index)
            .map(|(byte, _)| byte)
            .unwrap_or(text.len())
    };
    let start_byte = byte_index(start.min(end));
    let end_byte = byte_index(start.max(end));
    let mut result = String::with_capacity(text.len() + insertion.len());
    result.push_str(&text[..start_byte]);
    result.push_str(insertion);
    result.push_str(&text[end_byte..]);
    (result, start.min(end) + insertion.chars().count())
}

fn editor_line_count(text: &str) -> usize {
    text.split('\n').count()
}

use panel::PanelTab;

fn apply_theme(ctx: &egui::Context, theme: &str) {
    let light = theme == "light";
    let mut visuals = if theme == "light" {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let accent = if light {
        egui::Color32::from_rgb(38, 107, 157)
    } else {
        egui::Color32::from_rgb(97, 175, 239)
    };
    visuals.hyperlink_color = accent;
    visuals.selection.bg_fill = accent;
    visuals.selection.stroke.color = if light {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(17, 25, 35)
    };
    ctx.set_visuals(visuals);
}

fn language_for_path(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        _ => None,
    }
}

fn update_lsp_for_file(
    lsp: &mut LspPool,
    config: &Config,
    error_message: &mut Option<String>,
    path: &Path,
    text: &str,
) {
    let Some(language) = language_for_path(path) else {
        return;
    };
    if config.lsp.servers.contains_key(language) {
        if let Err(error) = lsp.change_document(language, path, text) {
            *error_message = Some(format!("Gagal mengirim perubahan ke LSP: {error}"));
        }
    }
}

fn lsp_position(text: &str, char_offset: usize) -> (u32, u32) {
    let mut line = 0;
    let mut character = 0;
    for ch in text.chars().take(char_offset) {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }
    (line, character)
}

fn text_offset_from_lsp(text: &str, target_line: u32, target_character: u32) -> usize {
    let mut line = 0;
    let mut character = 0;
    let mut offset = 0;
    for ch in text.chars() {
        if line > target_line {
            return offset;
        }
        if line == target_line {
            if ch == '\n' || character >= target_character {
                return offset;
            }
            character += ch.len_utf16() as u32;
        }
        offset += 1;
        if ch == '\n' {
            line += 1;
            character = 0;
        }
    }
    offset
}

fn replace_word(text: &str, cursor: usize, insertion: &str) -> (String, usize) {
    let cursor_byte = text
        .char_indices()
        .nth(cursor)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len());
    let prefix_start = text[..cursor_byte]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_')
        .last()
        .map(|(byte, _)| byte)
        .unwrap_or(cursor_byte);
    let suffix_end = text[cursor_byte..]
        .char_indices()
        .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '_')
        .last()
        .map(|(byte, ch)| cursor_byte + byte + ch.len_utf8())
        .unwrap_or(cursor_byte);
    let new_cursor = text[..prefix_start].chars().count() + insertion.chars().count();
    let mut result = String::with_capacity(text.len() + insertion.len());
    result.push_str(&text[..prefix_start]);
    result.push_str(insertion);
    result.push_str(&text[suffix_end..]);
    (result, new_cursor)
}

#[allow(dead_code)]
pub struct TyinApp {
    config: Config,
    events: EventBus,
    resources: Arc<ResourceManager>,
    project: Project,
    lsp: LspPool,
    tabs: Vec<Tab>,
    recent_files: Vec<PathBuf>,
    active_tab: Option<usize>,
    show_resource_monitor: bool,
    show_explorer: bool,
    show_panel: bool,
    show_palette: bool,
    palette_query: String,
    expanded_dirs: HashSet<PathBuf>,
    tree_cache: Vec<FsEntry>,
    watcher_started: bool,
    watcher_attempted: bool,
    watcher: Option<WatcherHandle>,
    workspace_scan: Option<Receiver<Result<(PathBuf, Vec<FsEntry>), String>>>,
    workspace_scan_replaces: bool,
    pending_workspace_scan: Option<(PathBuf, bool)>,
    workspace_loading: bool,
    external_changes: HashSet<PathBuf>,
    error_message: Option<String>,
    pending_close: Option<u64>,
    next_tab_id: u64,
    git_status: Option<GitStatus>,
    git_selected: Option<PathBuf>,
    git_diff: Option<String>,
    git_commit_message: String,
    confirm_git_commit: bool,
    panel_tab: PanelTab,
    terminal: Option<TerminalSession>,
    terminal_input: String,
    completion_items: Vec<LspCompletion>,
    completion_cursor: Option<usize>,
    completion_path: Option<PathBuf>,
    editor_id: Option<egui::Id>,
    pending_workspace: Option<PathBuf>,
    exit_confirmation: bool,
    confirmed_exit: bool,
}

impl TyinApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        apply_theme(&cc.egui_ctx, &config.ui.theme);
        Self {
            config: config.clone(),
            events: EventBus::default(),
            resources: Arc::new(ResourceManager::default()),
            project: Project::new(),
            lsp: LspPool::new(),
            tabs: Vec::new(),
            recent_files: config.editor.recent_files.clone(),
            active_tab: None,
            show_resource_monitor: config.resources.enable_monitor,
            show_explorer: true,
            show_panel: false,
            show_palette: false,
            palette_query: String::new(),
            expanded_dirs: HashSet::new(),
            tree_cache: Vec::new(),
            watcher_started: false,
            watcher_attempted: false,
            watcher: None,
            workspace_scan: None,
            workspace_scan_replaces: false,
            pending_workspace_scan: None,
            workspace_loading: false,
            external_changes: HashSet::new(),
            error_message: None,
            pending_close: None,
            next_tab_id: 1,
            git_status: None,
            git_selected: None,
            git_diff: None,
            git_commit_message: String::new(),
            confirm_git_commit: false,
            panel_tab: PanelTab::Output,
            terminal: None,
            terminal_input: String::new(),
            completion_items: Vec::new(),
            completion_cursor: None,
            completion_path: None,
            editor_id: None,
            pending_workspace: None,
            exit_confirmation: false,
            confirmed_exit: false,
        }
    }

    fn setup_lsp_for_file(&mut self, path: &Path, text: &str) {
        let Some((language, language_id)) = path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| match extension {
                "rs" => Some(("rust", "rust")),
                "json" => Some(("json", "json")),
                "toml" => Some(("toml", "toml")),
                _ => None,
            })
        else {
            return;
        };
        let Some(server) = self.config.lsp.servers.get(language).cloned() else {
            return;
        };
        let root = self
            .project
            .root
            .clone()
            .or_else(|| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let result = self
            .lsp
            .start(language, &server.command, &server.args, &root)
            .and_then(|()| self.lsp.open_document(language, language_id, path, text));
        if let Err(error) = result {
            self.error_message = Some(format!("Gagal menghubungkan LSP ({language}): {error}"));
        }
    }

    fn refresh_git_diff(&mut self) {
        let Some((root, path)) = self
            .project
            .root
            .as_deref()
            .zip(self.git_selected.as_deref())
        else {
            self.git_diff = None;
            return;
        };
        match tyin_git::diff(root, path) {
            Ok(diff) => self.git_diff = Some(diff),
            Err(error) => {
                self.git_diff = None;
                self.error_message = Some(format!("Gagal membaca diff Git: {error}"));
            }
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        let path = match path.canonicalize() {
            Ok(path) if path.is_file() => path,
            Ok(path) => {
                self.error_message =
                    Some(format!("Bukan file yang dapat dibuka: {}", path.display()));
                return;
            }
            Err(error) => {
                self.error_message = Some(format!("Gagal menemukan {}: {error}", path.display()));
                return;
            }
        };
        for (i, tab) in self.tabs.iter().enumerate() {
            if tab.path.as_ref() == Some(&path) {
                self.active_tab = Some(i);
                self.tabs[i].last_active = Instant::now();
                self.tabs[i].cached = false;
                self.completion_items.clear();
                self.completion_path = None;
                self.remember_file(&path);
                return;
            }
        }
        let buf = match Buffer::from_file(path.clone()) {
            Ok(buf) => buf,
            Err(error) => {
                self.error_message = Some(format!("Gagal membuka {}: {error}", path.display()));
                return;
            }
        };
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string();
        let edit_text = buf.as_string();
        let highlight_spans = syntax_view::spans(Some(&path), &edit_text);
        let syntax_info = syntax_view::info(Some(&path), &edit_text);
        self.setup_lsp_for_file(&path, &edit_text);
        let idx = self.tabs.len();
        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.wrapping_add(1);
        self.tabs.push(Tab {
            id,
            buffer: buf,
            path: Some(path.clone()),
            title,
            last_active: Instant::now(),
            cached: false,
            edit_text,
            highlight_spans,
            syntax_info,
            pending_cursor: None,
        });
        self.remember_file(&path);
        self.active_tab = Some(idx);
        self.completion_items.clear();
        self.completion_path = None;
        self.resources.inc_tabs();
    }

    fn remember_file(&mut self, path: &Path) {
        self.recent_files.retain(|recent| recent != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(12);
        self.config
            .editor
            .recent_files
            .clone_from(&self.recent_files);
        if let Err(error) = self.config.save("tyin.toml") {
            self.error_message = Some(format!("Gagal menyimpan file terbaru: {error}"));
        }
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs[idx].buffer.dirty {
            self.pending_close = Some(self.tabs[idx].id);
            return;
        }
        self.remove_tab(idx);
    }

    fn remove_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if let Some(path) = self.tabs[idx].path.as_deref() {
            if let Some(language) = language_for_path(path) {
                if self.lsp.touch(language) {
                    if let Err(error) = self.lsp.close_document(language, path) {
                        self.error_message = Some(format!("Gagal menutup dokumen LSP: {error}"));
                    }
                }
            }
        }
        self.tabs.remove(idx);
        self.resources.dec_tabs();
        match self.active_tab {
            Some(a) if a == idx => {
                self.active_tab = if self.tabs.is_empty() {
                    None
                } else {
                    Some(idx.min(self.tabs.len() - 1))
                };
            }
            Some(a) if a > idx => {
                self.active_tab = Some(a - 1);
            }
            _ => {}
        }
    }

    fn enforce_tab_policy(&mut self) {
        if !self.config.editor.lazy_tabs {
            return;
        }
        let idle = Duration::from_secs(self.config.editor.tab_unload_idle_secs);
        let now = Instant::now();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if Some(i) == self.active_tab {
                tab.cached = false;
                tab.last_active = now;
                continue;
            }
            if now.duration_since(tab.last_active) > idle && !tab.cached {
                tab.cached = true;
            }
        }
    }

    fn save_active(&mut self) -> bool {
        let Some(idx) = self.active_tab.filter(|idx| *idx < self.tabs.len()) else {
            return false;
        };
        self.save_tab(idx)
    }

    fn save_tab(&mut self, idx: usize) -> bool {
        if idx >= self.tabs.len() {
            return false;
        }
        let Some(path) = self.tabs[idx]
            .path
            .clone()
            .or_else(|| rfd::FileDialog::new().save_file())
        else {
            return false;
        };
        let new_text = self.tabs[idx].buffer.as_string();
        if let Err(error) = self.tabs[idx].buffer.save_to(path.clone()) {
            self.error_message = Some(format!("Gagal menyimpan {}: {error}", path.display()));
            return false;
        }

        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string();
        self.tabs[idx].path = Some(path.clone());
        self.tabs[idx].title = title;
        self.setup_lsp_for_file(&path, &new_text);
        self.external_changes.remove(&path);
        self.error_message = None;
        true
    }

    fn reload_active(&mut self) {
        let Some(idx) = self.active_tab.filter(|idx| *idx < self.tabs.len()) else {
            return;
        };
        self.reload_tab(idx);
    }

    fn reload_tab(&mut self, idx: usize) -> bool {
        if idx >= self.tabs.len() {
            return false;
        }
        let Some(path) = self.tabs[idx].path.clone() else {
            self.error_message = Some("File baru belum memiliki path untuk dimuat ulang.".into());
            return false;
        };
        match Buffer::from_file(path.clone()) {
            Ok(buf) => {
                self.tabs[idx].edit_text = buf.as_string();
                self.tabs[idx].highlight_spans =
                    syntax_view::spans(Some(&path), &self.tabs[idx].edit_text);
                self.tabs[idx].syntax_info =
                    syntax_view::info(Some(&path), &self.tabs[idx].edit_text);
                self.tabs[idx].buffer = buf;
                self.external_changes.remove(&path);
                self.error_message = None;
                let text = self.tabs[idx].edit_text.clone();
                update_lsp_for_file(
                    &mut self.lsp,
                    &self.config,
                    &mut self.error_message,
                    &path,
                    &text,
                );
                true
            }
            Err(error) => {
                self.error_message =
                    Some(format!("Gagal memuat ulang {}: {error}", path.display()));
                false
            }
        }
    }
}

impl eframe::App for TyinApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_workspace_scan();
        if ctx.input(|input| input.viewport().close_requested()) && !self.confirmed_exit {
            if self.has_unsaved_changes() {
                self.exit_confirmation = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        if self.watcher.is_none() && !self.watcher_attempted {
            if let Some(root) = self.project.root.clone() {
                let ev = self.events.clone();
                match watch_path(root, move |path| {
                    ev.send(Event::WorkspaceChanged);
                    ev.send(Event::FileChanged(path.to_string_lossy().to_string()));
                }) {
                    Ok(watcher) => {
                        self.watcher = Some(watcher);
                        self.watcher_started = true;
                        self.resources.inc_watchers();
                    }
                    Err(error) => {
                        self.watcher_attempted = true;
                        self.error_message = Some(format!("Gagal memantau workspace: {error}"));
                    }
                }
            }
        }

        let lsp_errors = self.lsp.poll_diagnostics();
        if !lsp_errors.is_empty() {
            self.error_message = Some(lsp_errors.join("\n"));
        }
        let active_language = self
            .active_tab
            .and_then(|idx| self.tabs.get(idx))
            .and_then(|tab| tab.path.as_deref())
            .and_then(language_for_path)
            .filter(|language| self.config.lsp.servers.contains_key(*language));
        if let Some(language) = active_language {
            self.lsp.touch(language);
        }
        let active_lsp = active_language
            .map(str::to_owned)
            .into_iter()
            .collect::<Vec<_>>();
        self.lsp.shutdown_idle(
            Duration::from_secs(self.config.lsp.idle_shutdown_secs),
            &active_lsp,
        );
        self.resources.set_lsp_count(self.lsp.count());

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::P)) {
            self.show_palette = !self.show_palette;
            if self.show_palette {
                self.palette_query.clear();
            }
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.save_active();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_palette = false;
        }

        self.enforce_tab_policy();
        let events: Vec<Event> = {
            let mut v = Vec::new();
            while let Some(ev) = self.events.try_recv() {
                v.push(ev);
            }
            v
        };
        for ev in events {
            match ev {
                Event::OpenFolder(p) => {
                    self.request_open_workspace(PathBuf::from(p));
                }
                Event::OpenFile(p) => {
                    self.open_file(PathBuf::from(p));
                }
                Event::CloseTab(i) => self.close_tab(i),
                Event::FocusTab(i) => {
                    if i < self.tabs.len() {
                        self.active_tab = Some(i);
                        self.tabs[i].last_active = Instant::now();
                        self.tabs[i].cached = false;
                        self.completion_items.clear();
                        self.completion_path = None;
                    }
                }
                Event::SaveActive => {
                    self.save_active();
                }
                Event::FileChanged(p) => {
                    let path = PathBuf::from(p);
                    if let Some(idx) = self
                        .tabs
                        .iter()
                        .position(|tab| tab.path.as_ref() == Some(&path))
                    {
                        if self.tabs[idx].buffer.dirty {
                            self.external_changes.insert(path);
                        } else {
                            self.reload_tab(idx);
                        }
                    }
                }
                Event::WorkspaceChanged => {
                    if !self.workspace_loading {
                        if let Some(root) = self.project.root.clone() {
                            self.refresh_workspace(root);
                        }
                    }
                }
                Event::ReloadActive => self.reload_active(),
                _ => {}
            }
        }

        egui::SidePanel::left("activity_bar")
            .resizable(false)
            .exact_width(48.0)
            .show(ctx, |ui| {
                let actions = activity_bar::show(
                    ui,
                    self.show_explorer,
                    self.show_resource_monitor,
                    self.show_panel,
                    self.show_panel && self.panel_tab == PanelTab::Git,
                    self.show_panel && self.panel_tab == PanelTab::Terminal,
                    self.show_palette,
                );
                for action in actions {
                    match action {
                        activity_bar::Action::Explorer => {
                            self.show_explorer = !self.show_explorer;
                        }
                        activity_bar::Action::Resources => {
                            self.show_resource_monitor = !self.show_resource_monitor;
                        }
                        activity_bar::Action::Panel => self.show_panel = !self.show_panel,
                        activity_bar::Action::Git => {
                            self.show_panel = true;
                            self.panel_tab = PanelTab::Git;
                            self.refresh_git_status();
                        }
                        activity_bar::Action::Terminal => {
                            self.show_panel = true;
                            self.panel_tab = PanelTab::Terminal;
                        }
                        activity_bar::Action::Palette => {
                            self.show_palette = !self.show_palette;
                            if self.show_palette {
                                self.palette_query.clear();
                            }
                        }
                        activity_bar::Action::Save => {
                            self.save_active();
                        }
                    }
                }
            });

        if self.show_explorer {
            egui::SidePanel::left("explorer")
                .resizable(true)
                .default_width(290.0)
                .show(ctx, |ui| {
                    match explorer::show(
                        ui,
                        self.project.root.as_deref(),
                        &self.tree_cache,
                        &mut self.expanded_dirs,
                        self.git_status.as_ref(),
                        self.workspace_loading,
                    ) {
                        Some(explorer::Action::OpenWorkspace) => {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.request_open_workspace(path);
                            }
                        }
                        Some(explorer::Action::ToggleDirectory(path)) => {
                            if self.expanded_dirs.contains(&path) {
                                self.expanded_dirs.remove(&path);
                            } else {
                                self.expanded_dirs.insert(path);
                            }
                        }
                        Some(explorer::Action::OpenFile(path)) => self.open_file(path),
                        None => {}
                    }
                });
        }

        egui::TopBottomPanel::top("title_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (mark, _) =
                    ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(mark, 5.0, egui::Color32::from_rgb(17, 25, 35));
                let mark_stroke = egui::Stroke::new(2.5_f32, egui::Color32::from_rgb(97, 175, 239));
                ui.painter().line_segment(
                    [
                        mark.left_top() + egui::vec2(5.0, 7.0),
                        mark.right_top() + egui::vec2(-5.0, 7.0),
                    ],
                    mark_stroke,
                );
                ui.painter().line_segment(
                    [
                        mark.center_top() + egui::vec2(0.0, 7.0),
                        mark.center_bottom() + egui::vec2(0.0, -5.0),
                    ],
                    mark_stroke,
                );
                ui.label("Tyin-Editor");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "theme:{} | LSP:{} | tabs:{}",
                        self.config.ui.theme,
                        self.lsp.count(),
                        self.tabs.len()
                    ));
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(message) = self.error_message.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                    if ui.small_button("Tutup").clicked() {
                        self.error_message = None;
                    }
                });
                ui.separator();
            }

            let changed_active_path = self
                .active_tab
                .and_then(|idx| self.tabs.get(idx))
                .and_then(|tab| tab.path.as_ref())
                .filter(|path| self.external_changes.contains(*path))
                .cloned();
            if let Some(path) = changed_active_path {
                ui.horizontal(|ui| {
                    ui.label("File changed on disk. Reload?");
                    if ui.button("Reload").clicked() {
                        if let Some(idx) = self
                            .tabs
                            .iter()
                            .position(|tab| tab.path.as_ref() == Some(&path))
                        {
                            self.reload_tab(idx);
                        }
                    }
                    if ui.button("Keep local").clicked() {
                        self.external_changes.remove(&path);
                    }
                });
                ui.separator();
            }

            if !self.tabs.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    let len = self.tabs.len();
                    for i in 0..len {
                        let is_active = self.active_tab == Some(i);
                        let title = self.tabs[i].title.clone();
                        let cached = self.tabs[i].cached;
                        let dirty = self.tabs[i].buffer.dirty;
                        let label = if is_active {
                            format!(
                                "[{}{}{}]",
                                title,
                                if cached { " (c)" } else { "" },
                                if dirty { "*" } else { "" }
                            )
                        } else {
                            format!(
                                "{}{}{}",
                                title,
                                if cached { "(c)" } else { "" },
                                if dirty { "*" } else { "" }
                            )
                        };
                        if ui.button(label).clicked() {
                            self.active_tab = Some(i);
                            self.tabs[i].last_active = Instant::now();
                            self.tabs[i].cached = false;
                            self.completion_items.clear();
                            self.completion_path = None;
                        }
                        if ui.small_button("x").clicked() {
                            self.close_tab(i);
                            break;
                        }
                    }
                });
                ui.separator();
            }
            if let Some(idx) = self.active_tab {
                if idx < self.tabs.len() {
                    if self.tabs[idx].cached {
                        ui.centered_and_justified(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.label("Tab cached (unloaded from active view)");
                                ui.label("Klik tab untuk aktifkan kembali");
                            });
                        });
                    } else {
                        let path_str = self.tabs[idx]
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "untitled".to_string());
                        let current_text = self.tabs[idx].edit_text.clone();
                        let spans = self.tabs[idx].highlight_spans.clone();
                        let syntax_status = self.tabs[idx].syntax_info.clone();
                        let line_count = editor_line_count(&current_text);
                        let editor_rows = line_count.saturating_add(EDITOR_EXTRA_SCROLL_LINES);
                        ui.horizontal(|ui| {
                            ui.label(&path_str);
                            if self.tabs[idx].buffer.dirty {
                                ui.label("*");
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(format!("{} | Ln {}", syntax_status, line_count));
                                },
                            );
                        });
                        ui.separator();
                        let mut completion_selection = None;
                        if self.completion_path.as_ref() == self.tabs[idx].path.as_ref()
                            && !self.completion_items.is_empty()
                        {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.label("Language server completions");
                                ui.horizontal_wrapped(|ui| {
                                    for completion in self.completion_items.clone() {
                                        if ui.button(&completion.label).clicked() {
                                            completion_selection = Some(completion);
                                        }
                                    }
                                });
                            });
                        }
                        let mut completion_request = None;
                        let mut definition_request = None;
                        egui::ScrollArea::both()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let line_height =
                                        ui.text_style_height(&egui::TextStyle::Monospace);
                                    let (gutter, _) = ui.allocate_exact_size(
                                        egui::vec2(36.0, editor_rows as f32 * line_height),
                                        egui::Sense::hover(),
                                    );
                                    let clip = ui.clip_rect();
                                    let first_line = ((clip.top() - gutter.top()) / line_height)
                                        .floor()
                                        .max(0.0)
                                        as usize;
                                    let last_line = (((clip.bottom() - gutter.top()) / line_height)
                                        .ceil()
                                        .max(0.0)
                                        as usize)
                                        .min(line_count);
                                    for line in first_line..last_line {
                                        ui.painter().text(
                                            egui::pos2(
                                                gutter.right() - 4.0,
                                                gutter.top() + line as f32 * line_height,
                                            ),
                                            egui::Align2::RIGHT_TOP,
                                            format!("{:>4}", line + 1),
                                            egui::FontId::monospace(14.0),
                                            ui.visuals().weak_text_color(),
                                        );
                                    }
                                    ui.separator();
                                    if let Some(tab) = self.tabs.get_mut(idx) {
                                        let tab_pressed = self.editor_id.is_some_and(|id| {
                                            ui.memory(|memory| memory.has_focus(id))
                                                && ui.input_mut(|input| {
                                                    input.consume_key(
                                                        egui::Modifiers::NONE,
                                                        egui::Key::Tab,
                                                    )
                                                })
                                        });
                                        let mut inserted_indent = false;
                                        if tab_pressed {
                                            if let Some(editor_id) = self.editor_id {
                                                if let Some(state) =
                                                    egui::TextEdit::load_state(ui.ctx(), editor_id)
                                                {
                                                    if let Some(range) = state.cursor.char_range() {
                                                        let spaces = " ".repeat(
                                                            self.config
                                                                .editor
                                                                .tab_size
                                                                .clamp(1, 16),
                                                        );
                                                        let (text, cursor) = replace_char_range(
                                                            &tab.edit_text,
                                                            range.primary.index,
                                                            range.secondary.index,
                                                            &spaces,
                                                        );
                                                        tab.edit_text = text;
                                                        tab.pending_cursor = Some(cursor);
                                                        inserted_indent = true;
                                                    }
                                                }
                                            }
                                        }
                                        let mut layouter =
                                            |ui: &egui::Ui, text: &str, wrap_width| {
                                                let mut job =
                                                    syntax_view::layout(text, &spans, wrap_width);
                                                job.wrap.max_width = wrap_width;
                                                ui.fonts(|fonts| fonts.layout_job(job))
                                            };
                                        let editor_width = ui.available_width();
                                        let output = egui::TextEdit::multiline(&mut tab.edit_text)
                                            .code_editor()
                                            .desired_rows(editor_rows)
                                            .desired_width(editor_width)
                                            .layouter(&mut layouter)
                                            .show(ui);
                                        let response = output.response;
                                        self.editor_id = Some(response.id);
                                        let cursor = output
                                            .cursor_range
                                            .map(|range| range.primary.ccursor.index);
                                        let applying_completion = completion_selection.is_some();

                                        if let Some(completion) = completion_selection.take() {
                                            if let Some(cursor) = self.completion_cursor {
                                                let (text, new_cursor) = replace_word(
                                                    &tab.edit_text,
                                                    cursor,
                                                    &completion.insert_text,
                                                );
                                                tab.edit_text = text;
                                                tab.buffer.set_text(&tab.edit_text);
                                                tab.pending_cursor = Some(new_cursor);
                                                tab.highlight_spans = syntax_view::spans(
                                                    tab.path.as_deref(),
                                                    &tab.edit_text,
                                                );
                                                tab.syntax_info = syntax_view::info(
                                                    tab.path.as_deref(),
                                                    &tab.edit_text,
                                                );
                                                if let Some(path) = tab.path.clone() {
                                                    update_lsp_for_file(
                                                        &mut self.lsp,
                                                        &self.config,
                                                        &mut self.error_message,
                                                        &path,
                                                        &tab.edit_text,
                                                    );
                                                }
                                            }
                                            self.completion_items.clear();
                                        } else if response.has_focus()
                                            && ui.input(|input| {
                                                input.modifiers.ctrl
                                                    && input.key_pressed(egui::Key::Space)
                                            })
                                        {
                                            if let (Some(path), Some(cursor)) =
                                                (tab.path.clone(), cursor)
                                            {
                                                if let Some(language) = language_for_path(&path) {
                                                    let (line, character) =
                                                        lsp_position(&tab.edit_text, cursor);
                                                    completion_request = Some((
                                                        language.to_string(),
                                                        path,
                                                        line,
                                                        character,
                                                        cursor,
                                                    ));
                                                }
                                            }
                                        } else if response.has_focus()
                                            && ui.input(|input| input.key_pressed(egui::Key::F12))
                                        {
                                            if let (Some(path), Some(cursor)) =
                                                (tab.path.clone(), cursor)
                                            {
                                                if let Some(language) = language_for_path(&path) {
                                                    let (line, character) =
                                                        lsp_position(&tab.edit_text, cursor);
                                                    definition_request = Some((
                                                        language.to_string(),
                                                        path,
                                                        line,
                                                        character,
                                                    ));
                                                }
                                            }
                                        }

                                        if response.changed() || inserted_indent {
                                            tab.buffer.set_text(&tab.edit_text);
                                            tab.highlight_spans = syntax_view::spans(
                                                tab.path.as_deref(),
                                                &tab.edit_text,
                                            );
                                            tab.syntax_info = syntax_view::info(
                                                tab.path.as_deref(),
                                                &tab.edit_text,
                                            );
                                            let lsp_update = tab
                                                .path
                                                .clone()
                                                .map(|path| (path, tab.edit_text.clone()));
                                            if let Some((path, text)) = lsp_update {
                                                update_lsp_for_file(
                                                    &mut self.lsp,
                                                    &self.config,
                                                    &mut self.error_message,
                                                    &path,
                                                    &text,
                                                );
                                            }
                                            if !applying_completion {
                                                self.completion_items.clear();
                                                self.completion_path = None;
                                            }
                                        }
                                        if let Some(cursor) = tab.pending_cursor.take() {
                                            if let Some(mut state) =
                                                egui::TextEdit::load_state(ui.ctx(), response.id)
                                            {
                                                state.cursor.set_char_range(Some(
                                                    egui::text::CCursorRange::one(
                                                        egui::text::CCursor::new(cursor),
                                                    ),
                                                ));
                                                state.store(ui.ctx(), response.id);
                                            }
                                        }
                                    }
                                });
                            });
                        if let Some((language, path, line, character, cursor)) = completion_request
                        {
                            match self.lsp.completions(&language, &path, line, character) {
                                Ok(items) => {
                                    self.completion_items = items;
                                    self.completion_cursor = Some(cursor);
                                    self.completion_path = Some(path);
                                }
                                Err(error) => {
                                    self.error_message =
                                        Some(format!("Gagal meminta completion LSP: {error}"));
                                }
                            }
                        }
                        if let Some((language, path, line, character)) = definition_request {
                            match self.lsp.definition(&language, &path, line, character) {
                                Ok(locations) => {
                                    if let Some(location) = locations.first() {
                                        match tyin_lsp::path_from_uri(&location.uri) {
                                            Ok(target) => {
                                                self.open_file(target.clone());
                                                if let Some(target_tab) =
                                                    self.tabs.iter_mut().find(|tab| {
                                                        tab.path.as_deref()
                                                            == Some(target.as_path())
                                                    })
                                                {
                                                    let text = &target_tab.edit_text;
                                                    target_tab.pending_cursor =
                                                        Some(text_offset_from_lsp(
                                                            text,
                                                            location.line,
                                                            location.character,
                                                        ));
                                                }
                                            }
                                            Err(error) => {
                                                self.error_message = Some(format!(
                                                    "Lokasi definisi LSP tidak valid: {error}"
                                                ));
                                            }
                                        }
                                    }
                                }
                                Err(error) => {
                                    self.error_message =
                                        Some(format!("Gagal meminta definisi LSP: {error}"));
                                }
                            }
                        }
                    }
                } else {
                    ui.centered_and_justified(|ui| ui.label("No active tab"));
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("Tyin-Editor");
                        ui.label("Native Rust • Resource-aware • No webview");
                        ui.add_space(8.0);
                        ui.label(
                            "Ctrl+P: Palette • Ctrl+S: Save • Ctrl+Z/Y: Undo/Redo • Open folder",
                        );
                    });
                });
            }
        });

        if let Some(tab_id) = self.pending_close {
            if let Some(idx) = self.tabs.iter().position(|tab| tab.id == tab_id) {
                let title = self.tabs[idx].title.clone();
                egui::Window::new("Perubahan belum disimpan")
                    .id(egui::Id::new("confirm_close_tab"))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Simpan perubahan pada \"{title}\" sebelum menutup?"
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Simpan").clicked() && self.save_tab(idx) {
                                self.pending_close = None;
                                self.remove_tab(idx);
                            }
                            if ui.button("Buang perubahan").clicked() {
                                self.pending_close = None;
                                self.remove_tab(idx);
                            }
                            if ui.button("Batal").clicked() {
                                self.pending_close = None;
                            }
                        });
                    });
            } else {
                self.pending_close = None;
            }
        }

        if let Some(path) = self.pending_workspace.clone() {
            egui::Window::new("Perubahan belum disimpan")
                .id(egui::Id::new("confirm_workspace_change"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Simpan perubahan sebelum membuka workspace lain?");
                    ui.horizontal(|ui| {
                        if ui.button("Simpan semua").clicked() && self.save_all_dirty() {
                            self.pending_workspace = None;
                            self.open_workspace(path.clone());
                        }
                        if ui.button("Buang perubahan").clicked() {
                            self.pending_workspace = None;
                            self.open_workspace(path.clone());
                        }
                        if ui.button("Batal").clicked() {
                            self.pending_workspace = None;
                        }
                    });
                });
        }

        if self.exit_confirmation {
            egui::Window::new("Perubahan belum disimpan")
                .id(egui::Id::new("confirm_application_exit"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Simpan perubahan sebelum keluar dari Tyin-Editor?");
                    ui.horizontal(|ui| {
                        if ui.button("Simpan semua").clicked() && self.save_all_dirty() {
                            self.exit_confirmation = false;
                            self.confirmed_exit = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Buang perubahan").clicked() {
                            self.exit_confirmation = false;
                            self.confirmed_exit = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Batal").clicked() {
                            self.exit_confirmation = false;
                        }
                    });
                });
        }

        let mut git_actions = Vec::new();
        if self.show_panel {
            egui::TopBottomPanel::bottom("panel")
                .resizable(true)
                .default_height(180.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(self.panel_tab == PanelTab::Output, "OUTPUT")
                            .clicked()
                        {
                            self.panel_tab = PanelTab::Output;
                        }
                        if ui
                            .selectable_label(self.panel_tab == PanelTab::Git, "GIT")
                            .clicked()
                        {
                            self.panel_tab = PanelTab::Git;
                        }
                        if ui
                            .selectable_label(self.panel_tab == PanelTab::Terminal, "TERMINAL")
                            .clicked()
                        {
                            self.panel_tab = PanelTab::Terminal;
                        }
                        if ui
                            .selectable_label(self.panel_tab == PanelTab::Problems, "PROBLEMS")
                            .clicked()
                        {
                            self.panel_tab = PanelTab::Problems;
                        }
                    });
                    ui.separator();
                    match self.panel_tab {
                        PanelTab::Git => {
                            git_actions = panel::show_git(
                                ui,
                                self.git_status.as_ref(),
                                self.git_selected.as_ref(),
                                &mut self.git_commit_message,
                                self.git_diff.as_deref(),
                            );
                        }
                        PanelTab::Terminal => {
                            if self.terminal.is_none() {
                                if ui.button("Start terminal").clicked() {
                                    let root = self
                                        .project
                                        .root
                                        .clone()
                                        .or_else(|| std::env::current_dir().ok());
                                    match root {
                                        Some(root) => match TerminalSession::spawn(&root) {
                                            Ok(terminal) => {
                                                self.terminal = Some(terminal);
                                                self.error_message = None;
                                            }
                                            Err(error) => {
                                                self.error_message = Some(format!(
                                                    "Gagal memulai terminal: {error}"
                                                ));
                                            }
                                        },
                                        None => {
                                            self.error_message = Some(
                                                "Direktori kerja terminal tidak tersedia.".into(),
                                            );
                                        }
                                    }
                                }
                            } else if let Some(terminal) = self.terminal.as_mut() {
                                let available = ui.available_size();
                                let cols = (available.x / 8.0).max(20.0) as u16;
                                let rows = (available.y / 18.0).max(5.0) as u16;
                                if let Err(error) = terminal.resize(cols, rows) {
                                    self.error_message =
                                        Some(format!("Gagal mengubah ukuran terminal: {error}"));
                                }
                                let output = terminal.colored_output();
                                egui::ScrollArea::vertical()
                                    .stick_to_bottom(true)
                                    .max_height((available.y - 36.0).max(80.0))
                                    .show(ui, |ui| {
                                        for line in output.iter().rev().take(200).rev() {
                                            ui.horizontal(|ui| {
                                                for segment in line {
                                                    if let Some([r, g, b]) = segment.color {
                                                        ui.colored_label(
                                                            egui::Color32::from_rgb(r, g, b),
                                                            egui::RichText::new(&segment.text)
                                                                .monospace(),
                                                        );
                                                    } else {
                                                        ui.monospace(&segment.text);
                                                    }
                                                }
                                            });
                                        }
                                    });
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut self.terminal_input)
                                        .hint_text("Ketik perintah lalu Enter"),
                                );
                                if ui.button("Send Ctrl+C").clicked() {
                                    if let Err(error) = terminal.write_input("\u{3}") {
                                        self.error_message = Some(format!(
                                            "Gagal mengirim interrupt terminal: {error}"
                                        ));
                                    }
                                }
                                let submit = response.has_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                                if submit {
                                    let mut command = std::mem::take(&mut self.terminal_input);
                                    command.push('\n');
                                    if let Err(error) = terminal.write_input(&command) {
                                        self.error_message =
                                            Some(format!("Gagal mengirim input terminal: {error}"));
                                    }
                                    response.request_focus();
                                }
                                match terminal.is_running() {
                                    Ok(true) => {}
                                    Ok(false) => {
                                        ui.label("Shell process has exited.");
                                    }
                                    Err(error) => {
                                        self.error_message =
                                            Some(format!("Gagal memeriksa terminal: {error}"));
                                    }
                                }
                            }
                        }
                        PanelTab::Problems => {
                            let diagnostics = self
                                .active_tab
                                .and_then(|idx| self.tabs.get(idx))
                                .and_then(|tab| tab.path.as_deref())
                                .map(|path| self.lsp.diagnostics_for(path))
                                .unwrap_or_default();
                            if diagnostics.is_empty() {
                                ui.label("No diagnostics for the active file.");
                            } else {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    for diagnostic in diagnostics {
                                        ui.label(format!(
                                            "{}:{}: {}",
                                            diagnostic.uri, diagnostic.line, diagnostic.message
                                        ));
                                    }
                                });
                            }
                        }
                        PanelTab::Output => {
                            ui.label("Tyin-Editor output.");
                        }
                    }
                });
        }
        for action in git_actions {
            let Some(root) = self.project.root.as_deref() else {
                self.error_message = Some("Tidak ada workspace Git yang terbuka.".into());
                continue;
            };
            match action {
                panel::GitAction::Select(path) => {
                    self.git_selected = Some(path);
                    self.refresh_git_diff();
                }
                panel::GitAction::Refresh => self.refresh_git_status(),
                panel::GitAction::Stage(path) => match tyin_git::stage(root, &path) {
                    Ok(()) => self.refresh_git_status(),
                    Err(error) => self.error_message = Some(format!("Gagal stage file: {error}")),
                },
                panel::GitAction::Unstage(path) => match tyin_git::unstage(root, &path) {
                    Ok(()) => self.refresh_git_status(),
                    Err(error) => self.error_message = Some(format!("Gagal unstage file: {error}")),
                },
                panel::GitAction::RequestCommit => self.confirm_git_commit = true,
            }
        }

        if self.confirm_git_commit {
            let staged_diff =
                self.project
                    .root
                    .as_deref()
                    .and_then(|root| match tyin_git::staged_diff(root) {
                        Ok(diff) => Some(diff),
                        Err(error) => {
                            self.error_message =
                                Some(format!("Gagal membaca staged diff: {error}"));
                            None
                        }
                    });
            egui::Window::new("Review staged changes")
                .id(egui::Id::new("confirm_git_commit"))
                .collapsible(false)
                .resizable(true)
                .default_size([760.0, 480.0])
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Commit message: {}",
                        self.git_commit_message.trim()
                    ));
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.monospace(
                            staged_diff
                                .as_deref()
                                .unwrap_or("Unable to load staged diff."),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Commit reviewed changes").clicked() {
                            match self.project.root.as_deref() {
                                Some(root) => {
                                    match tyin_git::commit(root, &self.git_commit_message) {
                                        Ok(()) => {
                                            self.confirm_git_commit = false;
                                            self.git_commit_message.clear();
                                            self.refresh_git_status();
                                        }
                                        Err(error) => {
                                            self.error_message =
                                                Some(format!("Git commit gagal: {error}"));
                                        }
                                    }
                                }
                                None => {
                                    self.error_message =
                                        Some("Tidak ada workspace Git yang terbuka.".into());
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_git_commit = false;
                        }
                    });
                });
        }

        if self.show_resource_monitor {
            egui::Window::new("Resource Monitor")
                .default_pos([ctx.screen_rect().width() - 350.0, 70.0])
                .default_size([330.0, 270.0])
                .resizable(true)
                .show(ctx, |ui| {
                    let s = self.resources.stats();
                    ui.label(format!("Uptime: {:?}", s.uptime));
                    ui.label(format!("Tabs open: {}", s.tabs_open));
                    ui.label(format!("LSP alive: {}", s.lsp_alive));
                    ui.label(format!("Watchers: {}", s.watchers));
                    ui.label(format!("Tree entries: {}", self.tree_cache.len()));
                    ui.separator();
                    let mut cached = 0u32;
                    let mut dirty = 0u32;
                    for t in self.tabs.iter() {
                        if t.cached { cached += 1; }
                        if t.buffer.dirty { dirty += 1; }
                    }
                    ui.label(format!("Tabs cached: {} | Dirty: {}", cached, dirty));
                    ui.label(format!("Lazy tabs: {} (idle {}s)", self.config.editor.lazy_tabs, self.config.editor.tab_unload_idle_secs));
                    ui.separator();
                    ui.label("Opts: lazy tabs • on-demand LSP • virtual scroll • ignore-aware • watcher • editable+save");
                });
        }

        if self.show_palette {
            egui::Window::new("Command Palette")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
                .default_size([600.0, 260.0])
                .show(ctx, |ui| {
                    ui.text_edit_singleline(&mut self.palette_query);
                    ui.separator();
                    if ui.button("> Save (Ctrl+S)").clicked() {
                        self.save_active();
                        self.show_palette = false;
                    }
                    if ui.button("> Open Folder").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.request_open_workspace(path);
                        }
                        self.show_palette = false;
                    }
                    if ui.button("> Reload Active File").clicked() {
                        self.reload_active();
                        self.show_palette = false;
                    }
                    if ui.button("> Toggle Resource Monitor").clicked() {
                        self.show_resource_monitor = !self.show_resource_monitor;
                        self.show_palette = false;
                    }
                    if ui.button("> Toggle Explorer").clicked() {
                        self.show_explorer = !self.show_explorer;
                        self.show_palette = false;
                    }
                    if ui.button("> Toggle Panel").clicked() {
                        self.show_panel = !self.show_panel;
                        self.show_palette = false;
                    }
                    if ui
                        .button(format!("> Toggle Theme ({})", self.config.ui.theme))
                        .clicked()
                    {
                        self.config.ui.theme = if self.config.ui.theme == "light" {
                            "dark".into()
                        } else {
                            "light".into()
                        };
                        apply_theme(ctx, &self.config.ui.theme);
                        if let Err(error) = self.config.save("tyin.toml") {
                            self.error_message =
                                Some(format!("Gagal menyimpan konfigurasi tema: {error}"));
                        }
                        self.show_palette = false;
                    }
                    ui.separator();
                    if !self.recent_files.is_empty() {
                        ui.label("Recently opened");
                        for path in self.recent_files.clone() {
                            let label = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            if ui
                                .button(format!(
                                    "{label} — {}",
                                    path.parent().unwrap_or(&path).display()
                                ))
                                .clicked()
                            {
                                self.open_file(path);
                                self.show_palette = false;
                                break;
                            }
                        }
                        ui.separator();
                    }
                    if !self.tree_cache.is_empty() && !self.palette_query.is_empty() {
                        let q = self.palette_query.to_lowercase();
                        let mut count = 0;
                        for entry in self.tree_cache.iter() {
                            if entry.is_dir {
                                continue;
                            }
                            let name = entry
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_lowercase();
                            if name.contains(&q) {
                                if ui
                                    .button(format!(
                                        "Open: {}",
                                        entry
                                            .path
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                    ))
                                    .clicked()
                                {
                                    self.open_file(entry.path.clone());
                                    self.show_palette = false;
                                    break;
                                }
                                count += 1;
                                if count > 12 {
                                    break;
                                }
                            }
                        }
                    }
                });
        }

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tyin-Editor v0.1.0 • native Rust • egui/wgpu");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(idx) = self.active_tab {
                        if idx < self.tabs.len() && self.tabs[idx].buffer.dirty {
                            ui.label("UNSAVED");
                        }
                    }
                    if self.watcher_started {
                        ui.label("watch:on");
                    }
                    ui.label(format!("LSP:{}", self.lsp.count()));
                });
            });
        });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        editor_line_count, lsp_position, replace_char_range, replace_word, text_offset_from_lsp,
    };

    #[test]
    fn counts_trailing_empty_lines_in_the_editor() {
        assert_eq!(editor_line_count(""), 1);
        assert_eq!(editor_line_count("one\ntwo"), 2);
        assert_eq!(editor_line_count("one\n"), 2);
    }

    #[test]
    fn converts_editor_offsets_to_lsp_utf16_positions() {
        assert_eq!(lsp_position("a😀\nβ", 2), (0, 3));
        assert_eq!(lsp_position("a😀\nβ", 4), (1, 1));
        assert_eq!(text_offset_from_lsp("a😀\nβ", 0, 3), 2);
        assert_eq!(text_offset_from_lsp("a😀\nβ", 1, 1), 4);
    }

    #[test]
    fn completion_replaces_the_word_at_the_cursor() {
        let (text, cursor) = replace_word("let print = 1", 6, "print");
        assert_eq!(text, "let print = 1");
        assert_eq!(cursor, 9);
    }

    #[test]
    fn indentation_replaces_the_selected_range() {
        let (text, cursor) = replace_char_range("alpha", 1, 3, "    ");
        assert_eq!(text, "a    ha");
        assert_eq!(cursor, 5);
    }
}
