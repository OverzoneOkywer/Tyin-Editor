use eframe::egui;
use std::path::PathBuf;
use tyin_git::GitStatus;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelTab {
    Output,
    Terminal,
    Problems,
    Git,
}

pub(super) enum GitAction {
    Select(PathBuf),
    Stage(PathBuf),
    Unstage(PathBuf),
    StageAll(Vec<PathBuf>),
    UnstageAll(Vec<PathBuf>),
    RequestCommit,
    Refresh,
}

pub(super) fn show_git(
    ui: &mut egui::Ui,
    status: Option<&GitStatus>,
    selected: Option<&PathBuf>,
    commit_message: &mut String,
    diff: Option<&str>,
) -> Vec<GitAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.heading("SOURCE CONTROL");
        if ui.button("Refresh").clicked() {
            actions.push(GitAction::Refresh);
        }
    });
    match status {
        Some(status) => {
            ui.label(format!("Branch: {}", status.branch));
            ui.label(format!("Changed files: {}", status.changed_files.len()));
            let staged: Vec<_> = status
                .changed_files
                .iter()
                .filter(|file| file.index_status != ' ' && file.index_status != '?')
                .map(|file| file.path.clone())
                .collect();
            let has_staged = !staged.is_empty();
            let unstaged: Vec<_> = status
                .changed_files
                .iter()
                .filter(|file| file.worktree_status != ' ' || file.index_status == '?')
                .map(|file| file.path.clone())
                .collect();
            ui.horizontal(|ui| {
                ui.strong(format!("STAGED · {}", staged.len()));
                if ui
                    .add_enabled(!staged.is_empty(), egui::Button::new("Unstage all"))
                    .clicked()
                {
                    actions.push(GitAction::UnstageAll(staged));
                }
            });
            for file in status
                .changed_files
                .iter()
                .filter(|file| file.index_status != ' ' && file.index_status != '?')
            {
                ui.horizontal(|ui| {
                    ui.monospace(format!("{}{}", file.index_status, file.worktree_status));
                    if ui
                        .selectable_label(
                            selected == Some(&file.path),
                            file.path.display().to_string(),
                        )
                        .clicked()
                    {
                        actions.push(GitAction::Select(file.path.clone()));
                    }
                    if ui.small_button("−").on_hover_text("Unstage file").clicked() {
                        actions.push(GitAction::Unstage(file.path.clone()));
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.strong(format!("CHANGES · {}", unstaged.len()));
                if ui
                    .add_enabled(!unstaged.is_empty(), egui::Button::new("Stage all"))
                    .clicked()
                {
                    actions.push(GitAction::StageAll(unstaged));
                }
            });
            egui::ScrollArea::vertical()
                .max_height(90.0)
                .show(ui, |ui| {
                    for file in &status.changed_files {
                        if file.worktree_status == ' ' && file.index_status != '?' {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.monospace(format!("{}{}", file.index_status, file.worktree_status));
                            if ui
                                .selectable_label(
                                    selected == Some(&file.path),
                                    file.path.display().to_string(),
                                )
                                .clicked()
                            {
                                actions.push(GitAction::Select(file.path.clone()));
                            }
                        });
                    }
                });
            if let Some(path) = selected {
                if let Some(file) = status.changed_files.iter().find(|file| &file.path == path) {
                    if (file.worktree_status != ' ' || file.index_status == '?')
                        && ui.button("Stage selected").clicked()
                    {
                        actions.push(GitAction::Stage(path.clone()));
                    }
                    if file.index_status != ' '
                        && file.index_status != '?'
                        && ui.button("Unstage selected").clicked()
                    {
                        actions.push(GitAction::Unstage(path.clone()));
                    }
                    ui.label("Diff review");
                    egui::ScrollArea::vertical()
                        .max_height(210.0)
                        .show(ui, |ui| {
                            ui.monospace(diff.unwrap_or("Select a file to load its diff."));
                        });
                }
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(commit_message)
                    .on_hover_text("Commit message");
                if ui
                    .add_enabled(
                        !commit_message.trim().is_empty() && has_staged,
                        egui::Button::new("Commit staged"),
                    )
                    .clicked()
                {
                    actions.push(GitAction::RequestCommit);
                }
            });
        }
        None => {
            ui.label("No Git repository detected in the workspace.");
        }
    }
    actions
}
