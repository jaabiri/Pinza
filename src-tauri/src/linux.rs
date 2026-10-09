//! Linux specifics: a layer-shell surface on Wayland compositors that offer
//! one (Hyprland, Sway, river, KDE…), and input regions so clicks pass
//! through everywhere except over the photos.

use crate::controller::{Rect, PANEL_HEIGHT};
use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tauri::WebviewWindow;

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("GDK_BACKEND").map_or(true, |b| b.contains("wayland"))
}

/// Turns the window into an overlay pinned to the top of the screen, under
/// any bar (Waybar keeps its space), that never takes keyboard focus.
/// Must run before the window is first shown.
pub fn init_layer_shell(win: &WebviewWindow) -> bool {
    if !is_wayland() || !gtk_layer_shell::is_supported() {
        return false;
    }
    let Ok(gtk) = win.gtk_window() else { return false };
    if gtk.is_realized() {
        gtk.unrealize();
    }
    gtk.init_layer_shell();
    gtk.set_namespace("pinza");
    gtk.set_layer(Layer::Overlay);
    gtk.set_anchor(Edge::Top, true);
    gtk.set_anchor(Edge::Left, true);
    gtk.set_anchor(Edge::Right, true);
    gtk.set_exclusive_zone(0);
    gtk.set_keyboard_mode(KeyboardMode::None);
    gtk.set_size_request(-1, PANEL_HEIGHT as i32);
    true
}

/// A layer surface only gets the keyboard when it asks for it.
pub fn set_keyboard(win: &WebviewWindow, take: bool) {
    let win = win.clone();
    let _ = win.clone().run_on_main_thread(move || {
        let Ok(gtk) = win.gtk_window() else { return };
        gtk.set_keyboard_mode(if take { KeyboardMode::Exclusive } else { KeyboardMode::None });
        if take {
            // The web view inside must hold the focus too, or keys go nowhere.
            gtk.present();
            if let Some(child) = gtk.child() {
                focus_deepest(&child);
            }
        }
    });
}

fn focus_deepest(widget: &gtk::Widget) {
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            focus_deepest(&child);
        }
    }
    if widget.can_focus() {
        widget.grab_focus();
    }
}

/// Only this part of the window takes the pointer. An empty list lets
/// everything through.
pub fn set_input_region(win: &WebviewWindow, rects: Vec<Rect>) {
    let win = win.clone();
    let _ = win.clone().run_on_main_thread(move || {
        let Ok(gtk) = win.gtk_window() else { return };
        let Some(gdk) = gtk.window() else { return };
        let region = gtk::cairo::Region::create();
        for r in rects {
            let rect = gtk::cairo::RectangleInt::new(
                r.x.floor() as i32,
                r.y.floor() as i32,
                r.w.ceil().max(1.0) as i32,
                r.h.ceil().max(1.0) as i32,
            );
            let _ = region.union_rectangle(&rect);
        }
        gdk.input_shape_combine_region(&region, 0, 0);
    });
}

/// Tells the controller when the pointer comes onto the line or leaves it.
/// Crossing events from GTK are reliable where the web view's are not.
pub fn watch_pointer(win: &WebviewWindow, app: tauri::AppHandle) {
    let Ok(gtk) = win.gtk_window() else { return };
    gtk.add_events(gtk::gdk::EventMask::ENTER_NOTIFY_MASK | gtk::gdk::EventMask::LEAVE_NOTIFY_MASK);
    let enter_app = app.clone();
    gtk.connect_enter_notify_event(move |_, ev| {
        if ev.detail() != gtk::gdk::NotifyType::Inferior {
            crate::controller::pointer(&enter_app, true);
        }
        gtk::glib::Propagation::Proceed
    });
    gtk.connect_leave_notify_event(move |_, ev| {
        if ev.detail() != gtk::gdk::NotifyType::Inferior {
            crate::controller::pointer(&app, false);
        }
        gtk::glib::Propagation::Proceed
    });
}

pub fn has(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}
