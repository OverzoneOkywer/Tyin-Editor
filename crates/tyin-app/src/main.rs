#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use tyin_core::Config;
use tyin_ui::TyinApp;

fn log_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,tyin_app=info"))
}

fn app_icon() -> egui::IconData {
    let size = 32_u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = if x < 10 {
                10_i32 - x as i32
            } else if x > 21 {
                x as i32 - 21
            } else {
                0
            };
            let dy = if y < 10 {
                10_i32 - y as i32
            } else if y > 21 {
                y as i32 - 21
            } else {
                0
            };
            let in_mark =
                (4..28).contains(&x) && (4..28).contains(&y) && dx.pow(2) + dy.pow(2) <= 36;
            let top = (8..11).contains(&y) && (9..23).contains(&x);
            let stem = (15..18).contains(&x) && (8..24).contains(&y);
            let dot = (x as i32 - 16).pow(2) + (y as i32 - 26).pow(2) <= 4;
            let color = if !in_mark {
                [0, 0, 0, 0]
            } else if dot {
                [152, 195, 121, 255]
            } else if top || stem {
                [97, 175, 239, 255]
            } else {
                [17, 25, 35, 255]
            };
            rgba.extend_from_slice(&color);
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .init();

    let config = match Config::load("tyin.toml") {
        Ok(config) => config,
        Err(error) => {
            tracing::error!("Failed to load tyin.toml: {error}");
            Config::default()
        }
    };
    tracing::info!("Tyin-Editor starting (native) - theme={}", config.ui.theme);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Tyin-Editor")
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Tyin-Editor",
        native_options,
        Box::new(move |cc| Ok(Box::new(TyinApp::new(cc, config)))),
    )
}

#[cfg(test)]
mod tests {
    use super::log_filter;

    #[test]
    fn default_log_filter_keeps_renderer_info_out_of_console() {
        assert_eq!(log_filter().to_string(), "tyin_app=info,warn");
    }
}
