use eframe::egui;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Explorer,
    Resources,
    Panel,
    Git,
    Terminal,
    Palette,
    Save,
}

#[derive(Clone, Copy)]
enum Icon {
    Explorer,
    Resources,
    Panel,
    Git,
    Terminal,
    Palette,
    Save,
}

pub(super) fn show(
    ui: &mut egui::Ui,
    explorer: bool,
    resources: bool,
    panel: bool,
    git: bool,
    terminal: bool,
    palette: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.vertical_centered(|ui| {
        ui.add_space(6.0);
        if draw(ui, Icon::Explorer, "Explorer", explorer) {
            actions.push(Action::Explorer);
        }
        ui.add_space(4.0);
        if draw(ui, Icon::Resources, "Resource monitor", resources) {
            actions.push(Action::Resources);
        }
        ui.add_space(4.0);
        if draw(ui, Icon::Panel, "Toggle panel", panel) {
            actions.push(Action::Panel);
        }
        ui.add_space(4.0);
        if draw(ui, Icon::Git, "Source control", git) {
            actions.push(Action::Git);
        }
        ui.add_space(4.0);
        if draw(ui, Icon::Terminal, "Terminal", terminal) {
            actions.push(Action::Terminal);
        }
        ui.add_space(4.0);
        if draw(ui, Icon::Palette, "Command palette", palette) {
            actions.push(Action::Palette);
        }
        ui.add_space(4.0);
        if draw(ui, Icon::Save, "Save active file", false) {
            actions.push(Action::Save);
        }
    });
    actions
}

fn draw(ui: &mut egui::Ui, icon: Icon, tooltip: &str, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::click());
    if active {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            7.0,
            ui.visuals().selection.bg_fill.gamma_multiply(0.16),
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
            1.0,
            ui.visuals().hyperlink_color,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 7.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let color = if active || response.hovered() {
        ui.visuals().hyperlink_color
    } else {
        ui.visuals().text_color()
    };
    let stroke = egui::Stroke::new(1.7_f32, color);
    let center = rect.center();
    let line = |painter: &egui::Painter, from, to| {
        painter.line_segment([from, to], stroke);
    };

    match icon {
        Icon::Explorer => {
            let folder = egui::Rect::from_center_size(center, egui::vec2(20.0, 14.0))
                .translate(egui::vec2(0.0, 3.0));
            ui.painter().rect_stroke(folder, 2.0, stroke);
            line(
                ui.painter(),
                folder.left_top(),
                folder.left_top() - egui::vec2(0.0, 3.0),
            );
            line(
                ui.painter(),
                folder.left_top() - egui::vec2(0.0, 3.0),
                folder.left_top() + egui::vec2(7.0, -3.0),
            );
            line(
                ui.painter(),
                folder.left_top() + egui::vec2(7.0, -3.0),
                folder.left_top() + egui::vec2(12.0, 0.0),
            );
        }
        Icon::Resources => {
            ui.painter().circle_stroke(center, 10.0, stroke);
            line(ui.painter(), center, center + egui::vec2(5.0, -6.0));
            line(
                ui.painter(),
                center + egui::vec2(-5.0, 7.0),
                center + egui::vec2(5.0, 7.0),
            );
        }
        Icon::Panel => {
            let panel = egui::Rect::from_center_size(center, egui::vec2(20.0, 16.0));
            ui.painter().rect_stroke(panel, 2.0, stroke);
            line(
                ui.painter(),
                panel.left_top() + egui::vec2(0.0, 5.0),
                panel.right_top() + egui::vec2(0.0, 5.0),
            );
        }
        Icon::Git => {
            let top = center + egui::vec2(-5.0, -8.0);
            let mid = center + egui::vec2(5.0, 0.0);
            let bottom = center + egui::vec2(-5.0, 8.0);
            line(ui.painter(), top, bottom);
            line(ui.painter(), top, mid);
            for point in [top, mid, bottom] {
                ui.painter().circle_filled(point, 2.5, color);
            }
        }
        Icon::Terminal => {
            let terminal = egui::Rect::from_center_size(center, egui::vec2(21.0, 16.0));
            ui.painter().rect_stroke(terminal, 2.0, stroke);
            line(
                ui.painter(),
                terminal.left_top() + egui::vec2(4.0, 5.0),
                terminal.left_top() + egui::vec2(7.0, 8.0),
            );
            line(
                ui.painter(),
                terminal.left_top() + egui::vec2(7.0, 8.0),
                terminal.left_top() + egui::vec2(4.0, 11.0),
            );
            line(
                ui.painter(),
                terminal.left_top() + egui::vec2(10.0, 11.0),
                terminal.left_top() + egui::vec2(15.0, 11.0),
            );
        }
        Icon::Palette => {
            for x in [-4.0, 4.0] {
                for y in [-4.0, 4.0] {
                    ui.painter()
                        .circle_filled(center + egui::vec2(x, y), 2.5, color);
                }
            }
        }
        Icon::Save => {
            let disk = egui::Rect::from_center_size(center, egui::vec2(18.0, 18.0));
            ui.painter().rect_stroke(disk, 2.0, stroke);
            let label = egui::Rect::from_center_size(
                disk.left_top() + egui::vec2(9.0, 5.0),
                egui::vec2(9.0, 5.0),
            );
            ui.painter().rect_stroke(label, 0.5, stroke);
            line(
                ui.painter(),
                disk.left_bottom() + egui::vec2(4.0, 0.0),
                disk.left_bottom() + egui::vec2(4.0, -6.0),
            );
            line(
                ui.painter(),
                disk.left_bottom() + egui::vec2(4.0, -6.0),
                disk.right_bottom() + egui::vec2(-4.0, -6.0),
            );
        }
    }

    response.on_hover_text(tooltip).clicked()
}
