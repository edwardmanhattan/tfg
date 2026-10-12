//! Draw the console theme's choice widgets so their geometry is measurable.
//! The theme body is a copy of `apply_ops_theme` (main.rs) — kept local so
//! this probe does not require exposing a binary-private fn.
use egui::ViewportBuilder;

fn apply_ops_theme(ctx: &egui::Context) {
    use tfg::tokens;
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let v = &mut style.visuals;
        v.window_fill = tokens::CONSOLE_NIGHT;
        v.window_stroke = egui::Stroke::new(1.0, tokens::HAIRLINE_SLATE);
        v.window_corner_radius = egui::CornerRadius::same(8);
        v.panel_fill = tokens::CONSOLE_NIGHT;
        v.faint_bg_color = tokens::PANEL_SLATE;
        v.extreme_bg_color = tokens::DEEP_WELL;
        v.hyperlink_color = tokens::RADAR_CYAN;
        v.selection.bg_fill = tokens::RADAR_CYAN;
        v.selection.stroke = egui::Stroke::new(1.0, tokens::DEEP_WELL);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.corner_radius = egui::CornerRadius::same(tokens::CONTROL_RADIUS);
            w.weak_bg_fill = tokens::BUTTON_GRAPHITE;
        }
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, tokens::HAIRLINE_SLATE);
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, tokens::CUT_GREY);
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, tokens::RADAR_CYAN);
        let cyan = tokens::RADAR_CYAN;
        v.widgets.hovered.weak_bg_fill =
            egui::Color32::from_rgba_unmultiplied(cyan.r(), cyan.g(), cyan.b(), 40);
        v.widgets.active.weak_bg_fill =
            egui::Color32::from_rgba_unmultiplied(cyan.r(), cyan.g(), cyan.b(), 70);
        style.spacing.item_spacing = tokens::ITEM_SPACING;
        style.spacing.button_padding = tokens::BUTTON_PADDING;
        style.spacing.indent = tokens::INDENT;
        use egui::TextStyle;
        style.text_styles.insert(TextStyle::Small, egui::FontId::proportional(tokens::TEXT_SMALL_PT));
        style.text_styles.insert(TextStyle::Body, egui::FontId::proportional(tokens::TEXT_BODY_PT));
        style.text_styles.insert(TextStyle::Button, egui::FontId::proportional(tokens::TEXT_BUTTON_PT));
        style.text_styles.insert(TextStyle::Heading, egui::FontId::proportional(tokens::TEXT_HEADING_PT));
        style.text_styles.insert(TextStyle::Monospace, egui::FontId::monospace(tokens::TEXT_MONO_PT));
    });
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([420.0, 220.0])
            .with_position([0.0, 0.0]),
        ..Default::default()
    };
    eframe::run_native(
        "choice-probe",
        opts,
        Box::new(|cc| {
            apply_ops_theme(&cc.egui_ctx);
            Ok(Box::new(App::default()))
        }),
    )
}

struct App {
    grid: bool,
    motion: bool,
    tog: bool,
    pick: i32,
}

impl Default for App {
    fn default() -> Self {
        App { grid: true, motion: true, tog: true, pick: 0 }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.checkbox(&mut self.grid, "Geographic grid");
            ui.toggle_value(&mut self.tog, "Reduce motion");
            ui.radio_value(&mut self.pick, 0, "option A");
            ui.radio_value(&mut self.pick, 1, "option B");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.motion, "compact");
                ui.label("inline");
            });
        });
        ui.ctx().request_repaint();
    }
}
