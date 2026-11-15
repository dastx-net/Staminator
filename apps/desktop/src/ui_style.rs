use egui::{Ui, vec2};

/// Shared spacing for secondary desktop windows. Colors follow the active egui theme.
pub fn apply(ui: &mut Ui) {
    let spacing = ui.spacing_mut();
    spacing.item_spacing = vec2(8.0, 8.0);
    spacing.button_padding = vec2(10.0, 6.0);
    spacing.interact_size.y = 30.0;
}
