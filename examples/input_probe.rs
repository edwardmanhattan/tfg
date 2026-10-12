//! Draw the theme's input widgets so their geometry is measurable. Theme
//! body copied from `apply_ops_theme` (main.rs) — local so no private fn
//! is exposed.
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
            .with_inner_size([460.0, 300.0])
            .with_position([0.0, 0.0]),
        ..Default::default()
    };
    eframe::run_native(
        "input-probe",
        opts,
        Box::new(|cc| {
            apply_ops_theme(&cc.egui_ctx);
            Ok(Box::new(App::default()))
        }),
    )
}

#[derive(Default)]
struct App {
    search: String,
    pwd: String,
    area: String,
    hint: String,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("singleline:");
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("search"));
            ui.label("password:");
            ui.add(egui::TextEdit::singleline(&mut self.pwd).password(true));
            ui.label("hint:");
            ui.add(egui::TextEdit::singleline(&mut self.hint).hint_text("type here"));
            ui.label("multiline:");
            ui.add(egui::TextEdit::multiline(&mut self.area).desired_rows(3));
        });
        ui.ctx().request_repaint();
    }
}
