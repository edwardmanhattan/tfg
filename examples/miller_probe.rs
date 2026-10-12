//! Draw the console's picker rows at the theme, at the shipped row height
//! (`picker_row_height`), so a Miller-column list is measurable. Theme body
//! copied from `apply_ops_theme` (main.rs) — local so no private fn exposed.
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

/// The shipped `picker_row_height`, verbatim (main.rs:146).
fn picker_row_height(ui: &egui::Ui) -> f32 {
    let mut scratch = egui::Ui::new(
        ui.ctx().clone(),
        ui.id().with("picker-row-probe"),
        egui::UiBuilder::new().max_rect(egui::Rect::ZERO),
    );
    use egui::Widget as _;
    let label = egui::Label::new(egui::RichText::new("M")).ui(&mut scratch);
    let button = egui::Button::new("M").ui(&mut scratch);
    label.rect.height().max(button.rect.height()) + ui.spacing().item_spacing.y
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([360.0, 320.0])
            .with_position([0.0, 0.0]),
        ..Default::default()
    };
    eframe::run_native(
        "miller-probe",
        opts,
        Box::new(|cc| {
            apply_ops_theme(&cc.egui_ctx);
            Ok(Box::new(App::default()))
        }),
    )
}

#[derive(Default)]
struct App {
    leaf: i32,
    cat: i32,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            let row = picker_row_height(ui);
            ui.label(format!("row height = {row:.2} pt"));
            ui.horizontal(|ui| {
                // taxonomy column (Miller left)
                ui.vertical(|ui| {
                    ui.weak("Category");
                    for (i, c) in ["Perang", "Angkut", "Latih"].iter().enumerate() {
                        ui.selectable_value(&mut self.cat, i as i32, *c);
                    }
                });
                ui.separator();
                // leaf column with thumbnail + name + weak subtitle
                ui.vertical(|ui| {
                    ui.weak("Pieces");
                    let rows = [
                        ("KRI Ahmad Yani", "frigate · 351"),
                        ("KRI Yos Sudarso", "frigate · 353"),
                        ("KRI Cakra", "SSK · 401"),
                        ("KRI Ardadedali", "SSK · 404"),
                    ];
                    for (i, (name, cls)) in rows.iter().enumerate() {
                        let armed = self.leaf == i as i32;
                        let r = ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(28.0, 28.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(
                                rect.center(),
                                14.0,
                                tfg::tokens::PANEL_SLATE,
                            );
                            ui.vertical(|ui| {
                                ui.label(*name);
                                ui.weak(*cls);
                            });
                            if armed {
                                ui.label(egui::RichText::new("armed").weak().small());
                            } else {
                                ui.small_button("assign");
                            }
                        });
                        if armed {
                            ui.painter().rect_stroke(
                                r.response.rect.expand(2.0),
                                egui::CornerRadius::same(3),
                                egui::Stroke::new(1.0, tfg::tokens::RADAR_CYAN),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                });
            });
        });
        ui.ctx().request_repaint();
    }
}
