use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tyin_fs::FsEntry;
use tyin_git::GitStatus;

pub(super) enum Action {
    OpenWorkspace,
    ToggleDirectory(PathBuf),
    OpenFile(PathBuf),
}

pub(super) fn show(
    ui: &mut egui::Ui,
    root: Option<&Path>,
    tree_cache: &[FsEntry],
    expanded_dirs: &mut HashSet<PathBuf>,
    git_status: Option<&GitStatus>,
    loading: bool,
    active_file: Option<&Path>,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.strong("EXPLORER");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Open folder").clicked() {
                action = Some(Action::OpenWorkspace);
            }
        });
    });
    ui.separator();
    if tree_cache.is_empty() {
        ui.label(if loading {
            "Scanning workspace..."
        } else {
            "No folder opened"
        });
        return action;
    }
    if loading {
        ui.weak("Scanning workspace...");
    }
    if let Some(root) = root {
        egui::Frame::none()
            .fill(ui.visuals().extreme_bg_color)
            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
            .rounding(6.0)
            .show(ui, |ui| {
                ui.strong(root.file_name().unwrap_or_default().to_string_lossy());
            });
    }
    let entries = visible_entries(root, tree_cache, expanded_dirs);
    ui.label(format!(
        "{} visible / {} total",
        entries.len(),
        tree_cache.len()
    ));
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show_rows(ui, row_height, entries.len(), |ui, range| {
            for i in range {
                let (entry, depth) = &entries[i];
                ui.horizontal(|ui| {
                    ui.add_space(*depth as f32 * 12.0);
                    if entry.is_dir {
                        let expanded = expanded_dirs.contains(&entry.path);
                        let marker = if expanded { "▾" } else { "▸" };
                        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                        if ui
                            .selectable_label(expanded, format!("{marker}  {name}"))
                            .clicked()
                        {
                            action = Some(Action::ToggleDirectory(entry.path.clone()));
                        }
                    } else {
                        let name = entry
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let git_marker = root
                            .and_then(|root| entry.path.strip_prefix(root).ok())
                            .and_then(|relative| {
                                git_status?
                                    .changed_files
                                    .iter()
                                    .find(|file| file.path == relative)
                            })
                            .map(|file| format!("{}{}", file.index_status, file.worktree_status));
                        if let Some(marker) = git_marker {
                            ui.colored_label(egui::Color32::from_rgb(209, 154, 102), marker);
                        } else {
                            ui.add_space(18.0);
                        }
                        if ui
                            .selectable_label(active_file == Some(entry.path.as_path()), name)
                            .clicked()
                        {
                            action = Some(Action::OpenFile(entry.path.clone()));
                        }
                    }
                });
            }
        });
    action
}

pub(super) fn visible_entries(
    root: Option<&Path>,
    entries: &[FsEntry],
    expanded_dirs: &HashSet<PathBuf>,
) -> Vec<(FsEntry, usize)> {
    let Some(root) = root else {
        return entries.iter().cloned().map(|entry| (entry, 0)).collect();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let relative = entry.path.strip_prefix(root).ok()?;
            let components: Vec<_> = relative.components().collect();
            let depth = components.len().saturating_sub(1);
            let mut parent = root.to_path_buf();
            let visible = components.iter().take(depth).all(|component| {
                parent.push(component.as_os_str());
                expanded_dirs.contains(&parent)
            });

            visible.then(|| (entry.clone(), depth))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::visible_entries;

    #[test]
    fn tree_hides_descendants_until_parent_is_expanded() {
        let root = PathBuf::from("/workspace");
        let entries = vec![
            FsEntry {
                path: root.join("README.md"),
                is_dir: false,
            },
            FsEntry {
                path: root.join("src"),
                is_dir: true,
            },
            FsEntry {
                path: root.join("src/main.rs"),
                is_dir: false,
            },
            FsEntry {
                path: root.join("src/nested"),
                is_dir: true,
            },
            FsEntry {
                path: root.join("src/nested/lib.rs"),
                is_dir: false,
            },
        ];
        let mut expanded_dirs = HashSet::new();

        let visible = visible_entries(Some(&root), &entries, &expanded_dirs);
        assert_eq!(
            visible
                .iter()
                .map(|(entry, _)| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![root.join("README.md"), root.join("src")]
        );

        expanded_dirs.insert(root.join("src"));
        let visible = visible_entries(Some(&root), &entries, &expanded_dirs);
        assert_eq!(
            visible
                .iter()
                .map(|(entry, _)| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![
                root.join("README.md"),
                root.join("src"),
                root.join("src/main.rs"),
                root.join("src/nested"),
            ]
        );
        assert_eq!(visible[2].1, 1);
    }

    use std::collections::HashSet;
    use std::path::PathBuf;
    use tyin_fs::FsEntry;
}
