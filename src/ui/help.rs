use eframe::egui;

pub fn show(ctx: &egui::Context, show: &mut bool) {
    egui::Window::new("Help")
        .open(show)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("epubthing");
            ui.add_space(8.0);
            ui.label("An EPUB reader written in Rust with egui.");
            ui.add_space(8.0);
            ui.label("Features:");
            ui.label("- Open EPUB files via drag-and-drop, File menu, or command line");
            ui.label("- Navigate chapters with side table of contents");
            ui.label("- Previous/Next with buttons or arrow keys");
            ui.label("- Auto-save last opened file");
            ui.label("- Dark/Light theme, custom text box colors");
            ui.add_space(8.0);
            ui.label("Shortcuts:");
            ui.label("  Left / Right arrow: change chapter");
            ui.label("  Ctrl+R: open recent files");
            ui.label("  Esc: close dialogs");
            ui.add_space(8.0);
            ui.label(format!("Version: {}", env!("CARGO_PKG_VERSION")));
        });
}
