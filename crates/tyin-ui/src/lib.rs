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
mod updater;
mod workbench;
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

#[derive(Clone, Copy)]
enum PaletteCommand {
    Save,
    OpenWorkspace,
    Reload,
    Files,
    Search,
    SourceControl,
    Room,
    Extensions,
    Terminal,
    ToggleTheme,
}

impl PaletteCommand {
    fn label(self, theme: &str) -> &'static str {
        match self {
            Self::Save => "Save active file",
            Self::OpenWorkspace => "Open workspace",
            Self::Reload => "Reload active file",
            Self::Files => "Show files",
            Self::Search => "Search workspace",
            Self::SourceControl => "Source control",
            Self::Room => "Open team room",
            Self::Extensions => "Browse extensions",
            Self::Terminal => "Toggle terminal",
            Self::ToggleTheme if theme == "light" => "Switch to dark theme",
            Self::ToggleTheme => "Switch to light theme",
        }
    }
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

fn gutter_line_numbers(
    row_ends_with_newline: impl IntoIterator<Item = bool>,
) -> Vec<Option<usize>> {
    let mut line_number = 1;
    let mut starts_line = true;
    row_ends_with_newline
        .into_iter()
        .map(|ends_with_newline| {
            let number = starts_line.then_some(line_number);
            if ends_with_newline {
                line_number += 1;
                starts_line = true;
            } else {
                starts_line = false;
            }
            number
        })
        .collect()
}

use panel::PanelTab;

fn apply_theme(ctx: &egui::Context, theme: &str) {
    let light = theme == "light";
    let mut visuals = if theme == "light" {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let (canvas, card, raised, hover, active, border, strong, body) = if light {
        (
            egui::Color32::from_rgb(241, 241, 241),
            egui::Color32::WHITE,
            egui::Color32::from_rgb(246, 246, 246),
            egui::Color32::from_rgb(239, 239, 239),
            egui::Color32::from_rgb(230, 230, 230),
            egui::Color32::from_rgb(228, 228, 228),
            egui::Color32::from_rgb(23, 23, 23),
            egui::Color32::from_rgb(79, 79, 79),
        )
    } else {
        (
            egui::Color32::from_rgb(8, 8, 8),
            egui::Color32::from_rgb(12, 12, 12),
            egui::Color32::from_rgb(17, 17, 17),
            egui::Color32::from_rgb(23, 23, 23),
            egui::Color32::from_rgb(29, 29, 29),
            egui::Color32::from_rgb(28, 28, 28),
            egui::Color32::from_rgb(244, 244, 244),
            egui::Color32::from_rgb(161, 161, 161),
        )
    };
    visuals.panel_fill = canvas;
    visuals.window_fill = card;
    visuals.extreme_bg_color = raised;
    visuals.faint_bg_color = card;
    visuals.code_bg_color = card;
    visuals.widgets.noninteractive.fg_stroke.color = body;
    visuals.widgets.noninteractive.bg_fill = card;
    visuals.widgets.noninteractive.bg_stroke.color = border;
    visuals.widgets.inactive.fg_stroke.color = body;
    visuals.widgets.inactive.bg_fill = raised;
    visuals.widgets.inactive.bg_stroke.color = border;
    visuals.widgets.hovered.fg_stroke.color = strong;
    visuals.widgets.hovered.bg_fill = hover;
    visuals.widgets.hovered.bg_stroke.color = border;
    visuals.widgets.active.fg_stroke.color = strong;
    visuals.widgets.active.bg_fill = active;
    visuals.widgets.active.bg_stroke.color = border;
    visuals.widgets.open.fg_stroke.color = strong;
    visuals.widgets.open.bg_fill = active;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.rounding = egui::Rounding::same(7.0);
    }
    visuals.hyperlink_color = body;
    visuals.selection.bg_fill = active;
    visuals.selection.stroke.color = strong;
    visuals.warn_fg_color = egui::Color32::from_rgb(194, 168, 116);
    visuals.error_fg_color = egui::Color32::from_rgb(190, 139, 139);
    visuals.window_rounding = egui::Rounding::same(12.0);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        style.spacing.interact_size = egui::vec2(24.0, 24.0);
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(11.5, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(9.5, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(15.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(12.5, egui::FontFamily::Monospace),
        );
    });
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
    repaint_ctx: egui::Context,
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
    file_filter: String,
    show_panel: bool,
    show_palette: bool,
    palette_query: String,
    palette_selection: usize,
    workspace_mode: workbench::Mode,
    search_state: workbench::SearchState,
    extension_query: String,
    extension_marketplace: bool,
    selected_extension: String,
    room_query: String,
    room_draft: String,
    room_messages: Vec<workbench::RoomMessage>,
    assistant_draft: String,
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
    updater: updater::UpdaterUi,
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
            repaint_ctx: cc.egui_ctx.clone(),
            config: config.clone(),
            events: EventBus::default(),
            resources: Arc::new(ResourceManager::default()),
            project: Project::new(),
            lsp: LspPool::new(),
            tabs: Vec::new(),
            recent_files: config.editor.recent_files.clone(),
            active_tab: None,
            show_resource_monitor: false,
            show_explorer: true,
            file_filter: String::new(),
            show_panel: false,
            show_palette: false,
            palette_query: String::new(),
            palette_selection: 0,
            workspace_mode: workbench::Mode::Files,
            search_state: workbench::SearchState::new(),
            extension_query: String::new(),
            extension_marketplace: false,
            selected_extension: "nova-lint".into(),
            room_query: String::new(),
            room_draft: String::new(),
            room_messages: workbench::initial_room_messages(),
            assistant_draft: String::new(),
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
            updater: updater::UpdaterUi::new(),
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
        self.updater.poll();
        if ctx.input(|input| input.viewport().close_requested()) && !self.confirmed_exit {
            if self.has_unsaved_changes() {
                self.exit_confirmation = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        if self.watcher.is_none() && !self.watcher_attempted {
            if let Some(root) = self.project.root.clone() {
                let ev = self.events.clone();
                let repaint_ctx = self.repaint_ctx.clone();
                match watch_path(root, move |path| {
                    ev.send(Event::WorkspaceChanged);
                    ev.send(Event::FileChanged(path.to_string_lossy().to_string()));
                    repaint_ctx.request_repaint();
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

        if ctx.input(|i| {
            i.modifiers.command && (i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::P))
        }) {
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
        if ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::F)) {
            self.workspace_mode = workbench::Mode::Search;
        }
        if ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::X)) {
            self.workspace_mode = workbench::Mode::Extensions;
        }
        if ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::R)) {
            self.workspace_mode = workbench::Mode::Room;
        }
        if ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::G)) {
            self.workspace_mode = workbench::Mode::SourceControl;
            self.refresh_git_status();
        }
        let focus_room_search = self.workspace_mode == workbench::Mode::Room
            && ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::F));

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

        let mut open_updates = false;
        egui::TopBottomPanel::top("title_bar")
            .exact_height(58.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("tyin.")
                            .strong()
                            .size(15.0)
                            .color(ui.visuals().strong_text_color()),
                    );
                    ui.separator();
                    let workspace = self
                        .project
                        .root
                        .as_deref()
                        .and_then(Path::file_name)
                        .unwrap_or_default()
                        .to_string_lossy();
                    let workspace_button = ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!("▣  {workspace}  ▾")).monospace(),
                        )
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke),
                    );
                    if workspace_button
                        .on_hover_text("Open the folder picker to change workspace")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.request_open_workspace(path);
                        }
                    }
                    if ui
                        .add_sized(
                            [ui.available_width().min(440.0), 34.0],
                            egui::Button::new(
                                egui::RichText::new("⌕  Jump to file, symbol or command     ⌘ K")
                                    .weak(),
                            ),
                        )
                        .clicked()
                    {
                        self.show_palette = true;
                        self.palette_query.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(self.updater.button_label()).clicked() {
                            open_updates = true;
                        }
                        let (run_fill, run_text) = if ui.visuals().dark_mode {
                            (
                                egui::Color32::from_rgb(237, 237, 237),
                                egui::Color32::from_rgb(9, 9, 9),
                            )
                        } else {
                            (
                                egui::Color32::from_rgb(26, 26, 26),
                                egui::Color32::from_rgb(250, 250, 250),
                            )
                        };
                        let _ = ui
                            .add_enabled(
                                false,
                                egui::Button::new(
                                    egui::RichText::new("▶  Run").strong().color(run_text),
                                )
                                .fill(run_fill),
                            )
                            .on_hover_text("Run targets are not configured yet.");
                        let _ = ui
                            .add_enabled(false, egui::Button::new("↗  Share"))
                            .on_hover_text("Workspace sharing is not available yet.");
                        ui.weak(format!("LSP {}", self.lsp.count()));
                    });
                });
            });
        if open_updates {
            self.updater.activate(ctx);
        }

        let open_files: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| explorer::OpenFile {
                title: tab.title.clone(),
                dirty: tab.buffer.dirty,
                active: self.active_tab == Some(index),
            })
            .collect();
        let mut git_actions = Vec::new();
        egui::SidePanel::left("activity_bar")
            .frame(egui::Frame::none())
            .resizable(false)
            .exact_width(52.0)
            .show(ctx, |ui| {
                let actions = egui::Frame::none()
                    .fill(ui.visuals().window_fill)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .rounding(egui::Rounding::same(16.0))
                    .outer_margin(egui::Margin::symmetric(3.0, 5.0))
                    .inner_margin(egui::Margin::symmetric(4.0, 8.0))
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        activity_bar::show(
                            ui,
                            self.workspace_mode,
                            self.show_resource_monitor,
                            self.show_panel && self.panel_tab == PanelTab::Terminal,
                        )
                    })
                    .inner;
                for action in actions {
                    match action {
                        activity_bar::Action::Mode(mode) => {
                            self.workspace_mode = mode;
                            self.show_explorer = true;
                            if mode == workbench::Mode::SourceControl {
                                self.refresh_git_status();
                            }
                        }
                        activity_bar::Action::Resources => {
                            self.show_resource_monitor = !self.show_resource_monitor;
                        }
                        activity_bar::Action::Terminal => {
                            self.show_panel = true;
                            self.panel_tab = PanelTab::Terminal;
                        }
                    }
                }
            });

        if self.show_explorer {
            egui::SidePanel::left("explorer")
                .frame(egui::Frame::none())
                .resizable(true)
                .default_width(258.0)
                .min_width(220.0)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(ui.visuals().window_fill)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .rounding(egui::Rounding::same(16.0))
                        .outer_margin(egui::Margin::symmetric(4.0, 5.0))
                        .inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            let active_file = self
                                .active_tab
                                .and_then(|index| self.tabs.get(index))
                                .and_then(|tab| tab.path.as_deref());
                            match self.workspace_mode {
                                workbench::Mode::Files | workbench::Mode::Room => {
                                    match explorer::show(
                                        ui,
                                        self.project.root.as_deref(),
                                        &self.tree_cache,
                                        &mut self.file_filter,
                                        &open_files,
                                        &mut self.expanded_dirs,
                                        self.git_status.as_ref(),
                                        self.workspace_loading,
                                        active_file,
                                    ) {
                                        Some(explorer::Action::OpenWorkspace) => {
                                            if let Some(path) = rfd::FileDialog::new().pick_folder()
                                            {
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
                                        Some(explorer::Action::OpenFile(path)) => {
                                            self.open_file(path)
                                        }
                                        Some(explorer::Action::FocusTab(index)) => {
                                            self.active_tab = Some(index);
                                            if let Some(tab) = self.tabs.get_mut(index) {
                                                tab.last_active = Instant::now();
                                                tab.cached = false;
                                            }
                                        }
                                        None => {}
                                    }
                                }
                                workbench::Mode::Search => {
                                    if let Some(hit) = self.search_state.show(
                                        ui,
                                        self.project.root.as_deref(),
                                        &self.tree_cache,
                                    ) {
                                        self.open_file(hit.path.clone());
                                        if let Some(tab) = self
                                            .tabs
                                            .iter_mut()
                                            .find(|tab| tab.path.as_ref() == Some(&hit.path))
                                        {
                                            tab.pending_cursor = Some(
                                                tab.edit_text
                                                    .split_inclusive('\n')
                                                    .take(hit.line.saturating_sub(1))
                                                    .map(|line| line.chars().count())
                                                    .sum(),
                                            );
                                        }
                                    }
                                }
                                workbench::Mode::SourceControl => {
                                    git_actions = panel::show_git(
                                        ui,
                                        self.git_status.as_ref(),
                                        self.git_selected.as_ref(),
                                        &mut self.git_commit_message,
                                        self.git_diff.as_deref(),
                                    );
                                }
                                workbench::Mode::Extensions => {
                                    workbench::show_extensions(
                                        ui,
                                        &mut self.extension_query,
                                        &mut self.extension_marketplace,
                                        &mut self.selected_extension,
                                    );
                                }
                            }
                        });
                });
        }

        if matches!(
            self.workspace_mode,
            workbench::Mode::Files | workbench::Mode::Room
        ) {
            egui::SidePanel::right("assistant_panel")
                .frame(egui::Frame::none())
                .resizable(true)
                .default_width(300.0)
                .min_width(260.0)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(ui.visuals().window_fill)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .rounding(egui::Rounding::same(16.0))
                        .outer_margin(egui::Margin::symmetric(4.0, 5.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            if self.workspace_mode == workbench::Mode::Room {
                                workbench::show_room(
                                    ui,
                                    &mut self.room_query,
                                    &mut self.room_draft,
                                    &mut self.room_messages,
                                    focus_room_search,
                                );
                            } else {
                                workbench::show_assistant(ui, &mut self.assistant_draft);
                            }
                        });
                });
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(ctx.style().visuals.window_fill)
                    .stroke(ctx.style().visuals.widgets.noninteractive.bg_stroke)
                    .rounding(egui::Rounding::same(16.0))
                    .outer_margin(egui::Margin::symmetric(4.0, 5.0))
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                if let Some(message) = self.error_message.clone() {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::LIGHT_RED, message);
                        if ui.small_button("Tutup").clicked() {
                            self.error_message = None;
                        }
                    });
                    ui.separator();
                }

                if self.workspace_mode == workbench::Mode::SourceControl {
                    workbench::show_diff(
                        ui,
                        self.git_diff.as_deref(),
                        self.git_selected.as_deref(),
                    );
                    return;
                }
                if self.workspace_mode == workbench::Mode::Extensions {
                    workbench::show_extension_detail(ui, &self.selected_extension);
                    return;
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
                    ui.horizontal(|ui| {
                        let len = self.tabs.len();
                        for i in 0..len {
                            let is_active = self.active_tab == Some(i);
                            let title = self.tabs[i].title.clone();
                            let cached = self.tabs[i].cached;
                            let dirty = self.tabs[i].buffer.dirty;
                            let mut close = false;
                            ui.horizontal(|ui| {
                                let mut label = title;
                                if cached {
                                    label.push_str("  · cached");
                                }
                                if dirty {
                                    label.push_str("  •");
                                }
                                if ui.selectable_label(is_active, label).clicked() {
                                    self.active_tab = Some(i);
                                    self.tabs[i].last_active = Instant::now();
                                    self.tabs[i].cached = false;
                                    self.completion_items.clear();
                                    self.completion_path = None;
                                }
                                close = ui.small_button("×").on_hover_text("Close tab").clicked();
                            });
                            if close {
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
                                .map(|path| {
                                    self.project
                                        .root
                                        .as_deref()
                                        .and_then(|root| path.strip_prefix(root).ok())
                                        .unwrap_or(path)
                                        .display()
                                        .to_string()
                                })
                                .unwrap_or_else(|| "untitled".to_string());
                            let (line_count, spans, syntax_status) = {
                                let tab = &self.tabs[idx];
                                (
                                    editor_line_count(&tab.edit_text),
                                    tab.highlight_spans.clone(),
                                    tab.syntax_info.clone(),
                                )
                            };
                            let editor_rows = line_count.saturating_add(EDITOR_EXTRA_SCROLL_LINES);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&path_str).monospace().small());
                                if self.tabs[idx].buffer.dirty {
                                    ui.colored_label(ui.visuals().warn_fg_color, "●");
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.weak(format!(
                                            "{}  ·  {} lines  ·  UTF-8",
                                            syntax_status, line_count
                                        ));
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
                                                    if let Some(state) = egui::TextEdit::load_state(
                                                        ui.ctx(),
                                                        editor_id,
                                                    ) {
                                                        if let Some(range) =
                                                            state.cursor.char_range()
                                                        {
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
                                                    let mut job = syntax_view::layout(
                                                        text,
                                                        &spans,
                                                        wrap_width,
                                                        self.config.ui.theme == "light",
                                                    );
                                                    job.wrap.max_width = wrap_width;
                                                    ui.fonts(|fonts| fonts.layout_job(job))
                                                };
                                            let editor_width = ui.available_width();
                                            let output =
                                                egui::TextEdit::multiline(&mut tab.edit_text)
                                                    .code_editor()
                                                    .desired_rows(editor_rows)
                                                    .desired_width(editor_width)
                                                    .layouter(&mut layouter)
                                                    .show(ui);
                                            let row_numbers = gutter_line_numbers(
                                                output
                                                    .galley
                                                    .rows
                                                    .iter()
                                                    .map(|row| row.ends_with_newline),
                                            );
                                            for (row, number) in
                                                output.galley.rows.iter().zip(row_numbers)
                                            {
                                                let Some(number) = number else {
                                                    continue;
                                                };
                                                let y = output.galley_pos.y + row.rect.min.y;
                                                if y + row.rect.height() < clip.top()
                                                    || y > clip.bottom()
                                                {
                                                    continue;
                                                }
                                                ui.painter().text(
                                                    egui::pos2(gutter.right() - 4.0, y),
                                                    egui::Align2::RIGHT_TOP,
                                                    format!("{number:>4}"),
                                                    egui::FontId::monospace(14.0),
                                                    ui.visuals().weak_text_color(),
                                                );
                                            }
                                            let response = output.response;
                                            self.editor_id = Some(response.id);
                                            let cursor = output
                                                .cursor_range
                                                .map(|range| range.primary.ccursor.index);
                                            let applying_completion =
                                                completion_selection.is_some();

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
                                                    if let Some(language) = language_for_path(&path)
                                                    {
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
                                                && ui.input(|input| {
                                                    input.key_pressed(egui::Key::F12)
                                                })
                                            {
                                                if let (Some(path), Some(cursor)) =
                                                    (tab.path.clone(), cursor)
                                                {
                                                    if let Some(language) = language_for_path(&path)
                                                    {
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
                                                if let Some(mut state) = egui::TextEdit::load_state(
                                                    ui.ctx(),
                                                    response.id,
                                                ) {
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
                            if let Some((language, path, line, character, cursor)) =
                                completion_request
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
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_width(360.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.heading("Tyin-Editor");
                                ui.weak("Native Rust · Resource-aware · No webview");
                                ui.add_space(12.0);
                                if ui.button("Open a workspace").clicked() {
                                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                        self.request_open_workspace(path);
                                    }
                                }
                                ui.add_space(6.0);
                                ui.weak("Ctrl+P  Command palette");
                                ui.weak("Ctrl+S  Save file");
                                ui.weak("Ctrl+Z / Ctrl+Y  Undo / redo");
                                ui.add_space(8.0);
                            });
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

        if self.show_panel && self.workspace_mode != workbench::Mode::SourceControl {
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
                                let output = match terminal.colored_output() {
                                    Ok(output) => output,
                                    Err(error) => {
                                        self.error_message =
                                            Some(format!("Gagal membaca layar terminal: {error}"));
                                        Vec::new()
                                    }
                                };
                                egui::ScrollArea::vertical()
                                    .stick_to_bottom(true)
                                    .max_height((available.y - 36.0).max(80.0))
                                    .show(ui, |ui| {
                                        for line in &output {
                                            ui.horizontal(|ui| {
                                                for segment in line {
                                                    let mut text =
                                                        egui::RichText::new(&segment.text)
                                                            .monospace();
                                                    if let Some([r, g, b]) = segment.color {
                                                        text = text.color(egui::Color32::from_rgb(
                                                            r, g, b,
                                                        ));
                                                    }
                                                    if let Some([r, g, b]) = segment.background {
                                                        text = text.background_color(
                                                            egui::Color32::from_rgb(r, g, b),
                                                        );
                                                    }
                                                    if segment.bold {
                                                        text = text.strong();
                                                    }
                                                    ui.label(text);
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
                panel::GitAction::StageAll(paths) => {
                    for path in paths {
                        if let Err(error) = tyin_git::stage(root, &path) {
                            self.error_message =
                                Some(format!("Gagal stage {}: {error}", path.display()));
                            break;
                        }
                    }
                    self.refresh_git_status();
                }
                panel::GitAction::UnstageAll(paths) => {
                    for path in paths {
                        if let Err(error) = tyin_git::unstage(root, &path) {
                            self.error_message =
                                Some(format!("Gagal unstage {}: {error}", path.display()));
                            break;
                        }
                    }
                    self.refresh_git_status();
                }
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

        let mut palette_action = None;
        if self.show_palette {
            let screen_rect = ctx.screen_rect();
            let backdrop_clicked = egui::Area::new(egui::Id::new("command_palette_backdrop"))
                .order(egui::Order::Middle)
                .fixed_pos(screen_rect.min)
                .show(ctx, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(180));
                    response.clicked()
                })
                .inner;
            if backdrop_clicked {
                self.show_palette = false;
            }
            egui::Window::new("Command palette")
                .id(egui::Id::new("tyin_command_palette"))
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .movable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 86.0))
                .default_size([568.0, 390.0])
                .frame(
                    egui::Frame::none()
                        .fill(ctx.style().visuals.window_fill)
                        .stroke(ctx.style().visuals.widgets.noninteractive.bg_stroke)
                        .rounding(egui::Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(12.0)),
                )
                .show(ctx, |ui| {
                    let query_response = ui.add_sized(
                        [ui.available_width(), 42.0],
                        egui::TextEdit::singleline(&mut self.palette_query)
                            .id(egui::Id::new("command_palette_query"))
                            .font(egui::TextStyle::Monospace)
                            .hint_text("Jump to file, symbol or command…"),
                    );
                    if !query_response.has_focus() && self.palette_query.is_empty() {
                        query_response.request_focus();
                    }
                    ui.separator();

                    let commands = [
                        PaletteCommand::Save,
                        PaletteCommand::OpenWorkspace,
                        PaletteCommand::Reload,
                        PaletteCommand::Files,
                        PaletteCommand::Search,
                        PaletteCommand::SourceControl,
                        PaletteCommand::Room,
                        PaletteCommand::Extensions,
                        PaletteCommand::Terminal,
                        PaletteCommand::ToggleTheme,
                    ];
                    let query = self.palette_query.to_lowercase();
                    let visible: Vec<_> = commands
                        .into_iter()
                        .filter(|command| {
                            command
                                .label(&self.config.ui.theme)
                                .to_lowercase()
                                .contains(&query)
                        })
                        .collect();
                    if !visible.is_empty() {
                        self.palette_selection = self.palette_selection.min(visible.len() - 1);
                        if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                            self.palette_selection =
                                (self.palette_selection + 1).min(visible.len() - 1);
                        }
                        if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
                            self.palette_selection = self.palette_selection.saturating_sub(1);
                        }
                        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
                            palette_action = visible.get(self.palette_selection).copied();
                        }
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for (index, command) in visible.iter().enumerate() {
                                    if ui
                                        .selectable_label(
                                            index == self.palette_selection,
                                            format!("⌘  {}", command.label(&self.config.ui.theme)),
                                        )
                                        .clicked()
                                    {
                                        palette_action = Some(*command);
                                    }
                                }
                            });
                    }

                    if visible.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(12.0);
                            ui.strong("No matching commands");
                            ui.weak("Try “search”, “theme”, or “workspace”.");
                        });
                    } else if !self.palette_query.is_empty() && !self.tree_cache.is_empty() {
                        ui.separator();
                        ui.label(egui::RichText::new("FILES").small().strong());
                        let query = query.clone();
                        let root = self.project.root.clone().unwrap_or_default();
                        let file_paths: Vec<_> = self
                            .tree_cache
                            .iter()
                            .filter(|entry| !entry.is_dir)
                            .filter(|entry| {
                                entry
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_lowercase()
                                    .contains(&query)
                            })
                            .take(5)
                            .map(|entry| entry.path.clone())
                            .collect();
                        egui::ScrollArea::vertical()
                            .max_height(110.0)
                            .show(ui, |ui| {
                                for path in file_paths {
                                    if ui
                                        .button(format!(
                                            "▤  {}",
                                            path.strip_prefix(&root).unwrap_or(&path).display()
                                        ))
                                        .clicked()
                                    {
                                        self.open_file(path);
                                        self.show_palette = false;
                                    }
                                }
                            });
                    }
                    ui.separator();
                    ui.weak("↑↓ navigate  ·  ↵ select  ·  esc close");
                });
        }
        if let Some(command) = palette_action {
            match command {
                PaletteCommand::Save => {
                    self.save_active();
                }
                PaletteCommand::OpenWorkspace => {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.request_open_workspace(path);
                    }
                }
                PaletteCommand::Reload => {
                    self.reload_active();
                }
                PaletteCommand::Files => self.workspace_mode = workbench::Mode::Files,
                PaletteCommand::Search => self.workspace_mode = workbench::Mode::Search,
                PaletteCommand::SourceControl => {
                    self.workspace_mode = workbench::Mode::SourceControl;
                    self.refresh_git_status();
                }
                PaletteCommand::Room => self.workspace_mode = workbench::Mode::Room,
                PaletteCommand::Extensions => self.workspace_mode = workbench::Mode::Extensions,
                PaletteCommand::Terminal => {
                    self.show_panel = !self.show_panel;
                    self.panel_tab = PanelTab::Terminal;
                }
                PaletteCommand::ToggleTheme => {
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
                }
            }
            self.show_palette = false;
        }

        let update_action = self.updater.show(ctx, self.has_unsaved_changes());
        match update_action {
            updater::InstallAction::None => {}
            updater::InstallAction::Download => self.updater.download_available(ctx),
            updater::InstallAction::Install { save_first } => {
                if save_first && !self.save_all_dirty() {
                    self.updater.request_install_confirmation();
                } else if self.updater.apply_install_action() {
                    self.exit_confirmation = false;
                    self.confirmed_exit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(34.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let branch = self
                        .git_status
                        .as_ref()
                        .map(|status| status.branch.as_str())
                        .unwrap_or("workspace");
                    ui.monospace(egui::RichText::new(format!("●  {branch}")).weak());
                    ui.weak("·");
                    ui.monospace(egui::RichText::new(format!("LSP {}", self.lsp.count())).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(idx) = self.active_tab {
                            if idx < self.tabs.len() && self.tabs[idx].buffer.dirty {
                                ui.label(egui::RichText::new("UNSAVED").weak().monospace());
                            }
                        }
                        if self.watcher_started {
                            ui.monospace(egui::RichText::new("watch:on").weak());
                        }
                        ui.monospace(
                            egui::RichText::new(format!("Tyin {}", env!("CARGO_PKG_VERSION")))
                                .weak(),
                        );
                        ui.monospace(
                            egui::RichText::new(format!("Spaces: {}", self.config.editor.tab_size))
                                .weak(),
                        );
                        ui.add_space(12.0);
                    });
                });
            });

        let has_active_work = self.workspace_scan.is_some()
            || self.lsp.count() > 0
            || (self.show_panel && self.panel_tab == PanelTab::Terminal && self.terminal.is_some());
        if has_active_work {
            ctx.request_repaint_after(Duration::from_millis(250));
        } else if self.show_resource_monitor {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_theme, editor_line_count, gutter_line_numbers, lsp_position, replace_char_range,
        replace_word, text_offset_from_lsp,
    };

    #[test]
    fn editor_themes_use_distinct_editor_and_panel_surfaces() {
        let context = egui::Context::default();
        apply_theme(&context, "dark");
        let dark = context.style().visuals.clone();
        assert!(dark.dark_mode);
        assert_ne!(dark.extreme_bg_color, dark.panel_fill);
        assert_eq!(dark.panel_fill, egui::Color32::from_rgb(8, 8, 8));
        assert_eq!(dark.window_fill, egui::Color32::from_rgb(12, 12, 12));
        assert_eq!(dark.hyperlink_color.r(), dark.hyperlink_color.g());
        assert_eq!(dark.hyperlink_color.g(), dark.hyperlink_color.b());

        apply_theme(&context, "light");
        let light = context.style().visuals.clone();
        assert!(!light.dark_mode);
        assert_eq!(light.panel_fill, egui::Color32::from_rgb(241, 241, 241));
        assert_eq!(light.window_fill, egui::Color32::WHITE);
        assert_ne!(light.extreme_bg_color, light.panel_fill);
    }

    #[test]
    fn counts_trailing_empty_lines_in_the_editor() {
        assert_eq!(editor_line_count(""), 1);
        assert_eq!(editor_line_count("one\ntwo"), 2);
        assert_eq!(editor_line_count("one\n"), 2);
    }

    #[test]
    fn gutter_numbers_logical_lines_across_wrapped_visual_rows() {
        assert_eq!(
            gutter_line_numbers([false, false, true, false, true, false]),
            [Some(1), None, None, Some(2), None, Some(3)]
        );
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
