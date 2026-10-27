use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use tyin_updater::Release;

enum UpdateEvent {
    Checked(Result<Option<Release>, String>),
    Downloaded(Release, Result<PathBuf, String>),
}

#[derive(Clone)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    Ready(Release, PathBuf),
    Installing,
    Unsupported,
    Error(String),
}

pub(super) struct UpdaterUi {
    state: UpdateState,
    sender: Sender<UpdateEvent>,
    receiver: Receiver<UpdateEvent>,
    show_window: bool,
    confirm_install: bool,
}

impl UpdaterUi {
    pub(super) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let supported = tyin_updater::installed_app_path().is_ok();
        Self {
            state: if supported {
                UpdateState::Idle
            } else {
                UpdateState::Unsupported
            },
            sender,
            receiver,
            show_window: false,
            confirm_install: false,
        }
    }

    pub(super) fn button_label(&self) -> String {
        match &self.state {
            UpdateState::Idle => "Check for updates".into(),
            UpdateState::Checking => "Checking updates…".into(),
            UpdateState::UpToDate => "Up to date".into(),
            UpdateState::Available(release) => format!("Update available · v{}", release.version),
            UpdateState::Downloading(release) => format!("Downloading v{}…", release.version),
            UpdateState::Ready(release, _) => format!("Install v{} and restart", release.version),
            UpdateState::Installing => "Installing update…".into(),
            UpdateState::Unsupported => "Updates unavailable".into(),
            UpdateState::Error(_) => "Retry update check".into(),
        }
    }

    pub(super) fn activate(&mut self, ctx: &egui::Context) {
        self.show_window = true;
        match self.state.clone() {
            UpdateState::Idle | UpdateState::UpToDate | UpdateState::Error(_) => {
                self.check(ctx);
            }
            UpdateState::Available(release) => self.download(release, ctx),
            UpdateState::Ready(_, _) => self.confirm_install = true,
            UpdateState::Checking
            | UpdateState::Downloading(_)
            | UpdateState::Installing
            | UpdateState::Unsupported => {}
        }
    }

    pub(super) fn download_available(&mut self, ctx: &egui::Context) {
        if let UpdateState::Available(release) = self.state.clone() {
            self.download(release, ctx);
        }
    }

    pub(super) fn request_install_confirmation(&mut self) {
        self.confirm_install = true;
    }

    pub(super) fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                UpdateEvent::Checked(Ok(Some(release))) => {
                    self.state = UpdateState::Available(release);
                }
                UpdateEvent::Checked(Ok(None)) => self.state = UpdateState::UpToDate,
                UpdateEvent::Checked(Err(error)) => self.state = UpdateState::Error(error),
                UpdateEvent::Downloaded(release, Ok(path)) => {
                    self.state = UpdateState::Ready(release, path);
                }
                UpdateEvent::Downloaded(_, Err(error)) => {
                    self.state = UpdateState::Error(error);
                }
            }
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, has_unsaved_changes: bool) -> InstallAction {
        let mut action = InstallAction::None;
        if self.show_window {
            egui::Window::new("Tyin-Editor updates")
                .id(egui::Id::new("tyin_update_window"))
                .collapsible(false)
                .resizable(true)
                .default_size([480.0, 320.0])
                .open(&mut self.show_window)
                .show(ctx, |ui| match &self.state {
                    UpdateState::Idle => {
                        ui.label("Check GitHub Releases for a newer Tyin-Editor version.");
                    }
                    UpdateState::Checking => {
                        ui.spinner();
                        ui.label("Checking the latest release…");
                    }
                    UpdateState::UpToDate => {
                        ui.label(format!(
                            "Tyin-Editor v{} is up to date.",
                            env!("CARGO_PKG_VERSION")
                        ));
                    }
                    UpdateState::Available(release) => {
                        ui.heading(format!("Version {} is available", release.version));
                        ui.label("Release notes:");
                        egui::ScrollArea::vertical()
                            .max_height(170.0)
                            .show(ui, |ui| {
                                ui.label(&release.notes);
                            });
                        if ui.button("Download update").clicked() {
                            action = InstallAction::Download;
                        }
                    }
                    UpdateState::Downloading(release) => {
                        ui.spinner();
                        ui.label(format!("Downloading and verifying v{}…", release.version));
                    }
                    UpdateState::Ready(release, _) => {
                        ui.heading(format!("Version {} is ready to install", release.version));
                        ui.label(
                            "The installer has been verified. The app will restart to apply it.",
                        );
                        if ui.button("Install and restart").clicked() {
                            if has_unsaved_changes {
                                self.confirm_install = true;
                            } else {
                                action = InstallAction::Install { save_first: false };
                            }
                        }
                    }
                    UpdateState::Installing => {
                        ui.spinner();
                        ui.label("The installer will run after Tyin-Editor closes.");
                    }
                    UpdateState::Unsupported => {
                        ui.label("In-app updates require the official Windows or Debian package.");
                    }
                    UpdateState::Error(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
        }

        if self.confirm_install {
            egui::Window::new("Unsaved changes")
                .id(egui::Id::new("confirm_update_install"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Save your changes before restarting to install the update?");
                    ui.horizontal(|ui| {
                        if ui.button("Save and install").clicked() {
                            action = InstallAction::Install { save_first: true };
                            self.confirm_install = false;
                        }
                        if ui.button("Discard and install").clicked() {
                            action = InstallAction::Install { save_first: false };
                            self.confirm_install = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_install = false;
                        }
                    });
                });
        }
        action
    }

    pub(super) fn apply_install_action(&mut self) -> bool {
        let UpdateState::Ready(_, installer) = &self.state else {
            return false;
        };
        let installer = installer.clone();
        let app = match tyin_updater::installed_app_path() {
            Ok(path) => path,
            Err(error) => {
                let _ = std::fs::remove_file(installer);
                self.state = UpdateState::Error(error.to_string());
                return false;
            }
        };
        match tyin_updater::launch_installer(&installer, &app) {
            Ok(()) => {
                self.state = UpdateState::Installing;
                true
            }
            Err(error) => {
                let _ = std::fs::remove_file(installer);
                self.state = UpdateState::Error(error.to_string());
                false
            }
        }
    }

    fn check(&mut self, ctx: &egui::Context) {
        self.state = UpdateState::Checking;
        let sender = self.sender.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = tyin_updater::check_for_update().map_err(|error| error.to_string());
            let _ = sender.send(UpdateEvent::Checked(result));
            repaint.request_repaint();
        });
    }

    fn download(&mut self, release: Release, ctx: &egui::Context) {
        self.state = UpdateState::Downloading(release.clone());
        let sender = self.sender.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result =
                tyin_updater::download_release(&release).map_err(|error| error.to_string());
            if let Err(error) = sender.send(UpdateEvent::Downloaded(release, result)) {
                if let UpdateEvent::Downloaded(_, Ok(path)) = error.0 {
                    let _ = std::fs::remove_file(path);
                }
            }
            repaint.request_repaint();
        });
    }
}

impl Drop for UpdaterUi {
    fn drop(&mut self) {
        if let UpdateState::Ready(_, path) = &self.state {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(super) enum InstallAction {
    None,
    Download,
    Install { save_first: bool },
}
