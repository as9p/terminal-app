//! The macOS menu bar. Tauri's default one gives Cmd+W to Close Window, which belongs to Close
//! pane here, so this keeps the default's items and swaps that one for Close tab on Cmd+Shift+W.

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Runtime};

const CLOSE_TAB: &str = "close_tab";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let app_menu = SubmenuBuilder::new(app, app.package_info().name.clone())
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let close_tab = MenuItemBuilder::with_id(CLOSE_TAB, "Close tab")
        .accelerator("CmdOrCtrl+Shift+W")
        .build(app)?;
    let file = SubmenuBuilder::new(app, "File").item(&close_tab).build()?;
    // The predefined items, so copy and paste work as before in the rename field.
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view = SubmenuBuilder::new(app, "View").fullscreen().build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;
    MenuBuilder::new(app)
        .items(&[&app_menu, &file, &edit, &view, &window])
        .build()
}

/// The window closes the tab itself, the same way as the key.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == CLOSE_TAB {
        let _ = app.emit("close-tab", ());
    }
}
