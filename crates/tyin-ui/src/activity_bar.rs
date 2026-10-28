use super::workbench::Mode;
use eframe::egui;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Mode(Mode),
    Terminal,
    Resources,
}

#[derive(Clone, Copy)]
enum Icon {
    Files,
    Search,
    Git,
    Room,
    Extensions,
    Terminal,
    Resources,
}

pub(super) fn show(ui: &mut egui::Ui, mode: Mode, resources: bool, terminal: bool) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.vertical_centered(|ui| {
        ui.add_space(10.0);
        for (icon, label, selected) in [
            (Icon::Files, "Files", mode == Mode::Files),
            (Icon::Search, "Search", mode == Mode::Search),
            (Icon::Git, "Source control", mode == Mode::SourceControl),
            (Icon::Room, "Team room", mode == Mode::Room),
            (Icon::Extensions, "Extensions", mode == Mode::Extensions),
        ] {
            if draw(ui, icon, label, selected) {
                let selected_mode = match icon {
                    Icon::Files => Mode::Files,
                    Icon::Search => Mode::Search,
                    Icon::Git => Mode::SourceControl,
                    Icon::Room => Mode::Room,
                    Icon::Extensions => Mode::Extensions,
                    Icon::Terminal | Icon::Resources => unreachable!(),
                };
                actions.push(Action::Mode(selected_mode));
            }
            ui.add_space(5.0);
        }

        ui.separator();
        if draw(ui, Icon::Terminal, "Terminal", terminal) {
            actions.push(Action::Terminal);
        }
        if draw(ui, Icon::Resources, "Resource monitor", resources) {
            actions.push(Action::Resources);
        }
        ui.with_layout(
            egui::Layout::top_down(egui::Align::Center).with_main_justify(true),
            |ui| {
                ui.add_space(ui.available_height().max(0.0) - 74.0);
                let _ = ui.small_button("?").on_hover_text("Help");
                ui.add_space(4.0);
                let avatar = ui.add_sized(
                    [30.0, 30.0],
                    egui::Button::new(egui::RichText::new("TY").small().strong())
                        .fill(ui.visuals().widgets.inactive.bg_fill),
                );
                avatar.on_hover_text("Tyin-Editor");
            },
        );
    });
    actions
}

fn draw(ui: &mut egui::Ui, icon: Icon, tooltip: &str, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
    let (active_fill, active_icon) = if ui.visuals().dark_mode {
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
    if active {
        ui.painter().rect_filled(rect.shrink(1.0), 8.0, active_fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 8.0, ui.visuals().widgets.hovered.bg_fill);
    }

    let color = if active {
        active_icon
    } else {
        ui.visuals().weak_text_color()
    };
    let stroke = egui::Stroke::new(1.5_f32, color);
    let center = rect.center();
    let line = |painter: &egui::Painter, from, to| {
        painter.line_segment([from, to], stroke);
    };
    let box_icon = |painter: &egui::Painter, size, radius| {
        painter.rect_stroke(egui::Rect::from_center_size(center, size), radius, stroke);
    };

    match icon {
        Icon::Files => {
            let folder = egui::Rect::from_center_size(center, egui::vec2(18.0, 13.0))
                .translate([0.0, 2.0].into());
            ui.painter().rect_stroke(folder, 2.0, stroke);
            line(
                ui.painter(),
                folder.left_top(),
                folder.left_top() + egui::vec2(0.0, -3.0),
            );
            line(
                ui.painter(),
                folder.left_top() + egui::vec2(0.0, -3.0),
                folder.left_top() + egui::vec2(7.0, -3.0),
            );
        }
        Icon::Search => {
            ui.painter()
                .circle_stroke(center - egui::vec2(2.0, 2.0), 6.0, stroke);
            line(
                ui.painter(),
                center + egui::vec2(3.0, 3.0),
                center + egui::vec2(8.0, 8.0),
            );
        }
        Icon::Git => {
            let top = center + egui::vec2(-5.0, -8.0);
            let middle = center + egui::vec2(5.0, 0.0);
            let bottom = center + egui::vec2(-5.0, 8.0);
            line(ui.painter(), top, bottom);
            line(ui.painter(), top, middle);
            for point in [top, middle, bottom] {
                ui.painter().circle_filled(point, 2.0, color);
            }
        }
        Icon::Room => {
            box_icon(ui.painter(), egui::vec2(18.0, 14.0), 4.0);
            line(
                ui.painter(),
                center + egui::vec2(-4.0, 7.0),
                center + egui::vec2(-7.0, 10.0),
            );
            line(
                ui.painter(),
                center + egui::vec2(-7.0, 10.0),
                center + egui::vec2(-7.0, 6.0),
            );
            for x in [-4.0, 0.0, 4.0] {
                ui.painter()
                    .circle_filled(center + egui::vec2(x, 0.0), 1.0, color);
            }
        }
        Icon::Extensions => {
            box_icon(ui.painter(), egui::vec2(14.0, 14.0), 3.0);
            ui.painter()
                .circle_filled(center + egui::vec2(0.0, -7.0), 2.0, color);
            ui.painter()
                .circle_filled(center + egui::vec2(7.0, 0.0), 2.0, color);
        }
        Icon::Terminal => {
            box_icon(ui.painter(), egui::vec2(18.0, 14.0), 3.0);
            line(
                ui.painter(),
                center + egui::vec2(-5.0, -2.0),
                center + egui::vec2(-2.0, 1.0),
            );
            line(
                ui.painter(),
                center + egui::vec2(-2.0, 1.0),
                center + egui::vec2(-5.0, 4.0),
            );
            line(
                ui.painter(),
                center + egui::vec2(1.0, 4.0),
                center + egui::vec2(5.0, 4.0),
            );
        }
        Icon::Resources => {
            ui.painter().circle_stroke(center, 8.0, stroke);
            line(ui.painter(), center, center + egui::vec2(4.0, -5.0));
        }
    }

    response.on_hover_text(tooltip).clicked()
}
