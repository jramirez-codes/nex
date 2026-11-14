//! Pointer and keyboard input: clicks, touch gestures (long press, pan), dragging files,
//! and the commands menus and buttons issue.

use std::path::PathBuf;
use std::time::Duration;

use egui::{DragAndDrop, Key, Modifiers, Pos2, Rect, Ui, Vec2};

use super::items::Layout;
use super::transfer::set_clipboard;
use super::{FileExplorer, View};

/// Holding a touch this long (without moving) starts selecting.
pub(super) const LONG_PRESS: f64 = 0.45;
pub(super) const DOUBLE_CLICK: f64 = 0.4;

/// Drag-and-drop payload for files dragged out of an explorer.
pub struct ExplorerDrag {
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Gesture {
    /// Pressed, not yet moved: may become a click, a long press, a pan or a drag.
    Pending,
    /// A touch was held: the item is selected and a following move drags the selection.
    LongPressed,
    /// A touch drag scrolling the listing.
    Panning,
    /// Files are being dragged (the payload is in `egui::DragAndDrop`).
    Dragging,
    /// Nothing more to do for this press.
    Idle,
}

pub(super) struct Press {
    pub(super) item: Option<usize>,
    pub(super) origin: Pos2,
    pub(super) at: f64,
    pub(super) touch: bool,
    pub(super) gesture: Gesture,
}

/// Things the right-click menu or the action bar asked for (applied after drawing).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Command {
    Open(usize),
    Copy,
    Cut,
    Paste,
    PasteInto(usize),
    ClearClipboard,
    CopyPath,
    Delete,
    SelectMode,
    SelectAll,
    ClearSelection,
    ToggleView,
    ToggleHidden,
}

/// This frame's pointer state, read once.
pub(super) struct PointerInput {
    pub(super) pos: Option<Pos2>,
    pub(super) pressed: bool,
    pub(super) released: bool,
    pub(super) down: bool,
    pub(super) delta: Vec2,
    pub(super) mods: Modifiers,
    pub(super) touches: bool,
    /// Two or more fingers are down (a pinch): the press is no longer a tap, pan or drag.
    pub(super) pinching: bool,
    pub(super) now: f64,
}

/// Where the primary button was released this frame: the event's own position. A touch
/// release can be followed in the same frame by a pointer move to wherever the (hidden) mouse
/// cursor sits (sway does this when the finger lifts), which `interact_pos` would report.
pub fn release_pos(i: &egui::InputState) -> Option<Pos2> {
    i.raw.events.iter().rev().find_map(|e| match e {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => Some(*pos),
        _ => None,
    })
}

impl PointerInput {
    /// Read the pointer. On release the position is where the button (or finger) came up;
    /// a touch release also reports the pointer as gone, so it falls back to `last_pos`
    /// (which this keeps up to date).
    pub(super) fn read(ui: &Ui, last_pos: &mut Option<Pos2>) -> Self {
        let mut input = ui.input(|i| PointerInput {
            pos: release_pos(i).or(i.pointer.interact_pos()),
            pressed: i.pointer.primary_pressed(),
            released: i.pointer.primary_released(),
            down: i.pointer.primary_down(),
            delta: i.pointer.delta(),
            mods: i.modifiers,
            touches: i.any_touches(),
            pinching: i.multi_touch().is_some(),
            now: i.time,
        });
        if input.released {
            input.pos = input.pos.or(*last_pos);
        }
        if input.pos.is_some() {
            *last_pos = input.pos;
        }
        input
    }
}

impl FileExplorer {
    /// Follow a press from start to finish: it may become a click, a long press (touch), a
    /// pan (touch) or a drag of files. A press starts inside the list; the rest of it is
    /// tracked even outside.
    pub(super) fn track_press(&mut self, ctx: &egui::Context, input: &PointerInput, layout: &Layout, list: Rect) {
        // A dialog (the delete confirmation) is on top: the listing doesn't take presses.
        if ctx.memory(|m| m.top_modal_layer()).is_some() || self.confirm_delete.is_some() {
            self.press = None;
            return;
        }
        if input.pressed && input.pos.is_some_and(|p| list.contains(p)) {
            let p = input.pos.unwrap_or_default();
            self.touch = input.touches;
            let item = layout.item_at(p, self.scroll, self.entries.len());
            self.press = Some(Press { item, origin: p, at: input.now, touch: input.touches, gesture: Gesture::Pending });
        }
        if input.pinching {
            // The first finger of a pinch also arrives as a press; it must not pan, open,
            // start selecting or drag. `Idle` keeps it inert until it lifts.
            if let Some(press) = self.press.as_mut() {
                press.gesture = Gesture::Idle;
            }
        }
        if let (Some(p), true) = (input.pos, input.down) {
            self.advance_gesture(ctx, p, input);
        }
        if input.released {
            if let Some(press) = self.press.take() {
                if press.gesture == Gesture::Pending {
                    self.click(press.item, press.touch, input.mods, input.now);
                }
            }
        }
    }

    /// While the button is held: decide what the press is, or keep doing it.
    fn advance_gesture(&mut self, ctx: &egui::Context, p: Pos2, input: &PointerInput) {
        // While selecting, a touch drag that starts on a selected item moves the selection
        // (instead of scrolling), so a long press isn't needed again.
        let on_marked = self.selecting && self.press.as_ref().and_then(|p| p.item).is_some_and(|i| self.is_marked(i));
        let Some(press) = self.press.as_mut() else { return };
        let moved = (p - press.origin).length();
        let slop = if press.touch { 10.0 } else { 6.0 };
        match press.gesture {
            Gesture::Pending if moved > slop => {
                if press.touch && on_marked {
                    press.gesture = Gesture::Dragging;
                    let i = press.item.unwrap_or_default();
                    self.start_drag(ctx, i);
                } else if press.touch {
                    press.gesture = Gesture::Panning;
                    self.scroll -= p.y - press.origin.y;
                } else if let Some(i) = press.item {
                    press.gesture = Gesture::Dragging;
                    self.start_drag(ctx, i);
                } else {
                    press.gesture = Gesture::Idle;
                }
            }
            Gesture::Pending if press.touch && press.item.is_some() => {
                let left = LONG_PRESS - (input.now - press.at);
                if left > 0.0 {
                    ctx.request_repaint_after(Duration::from_secs_f64(left));
                    return;
                }
                press.gesture = Gesture::LongPressed;
                let i = press.item.unwrap_or_default();
                self.selecting = true;
                self.selected = i;
                if !self.is_marked(i) {
                    self.toggle_mark(i);
                }
            }
            Gesture::LongPressed if moved > slop => {
                press.gesture = Gesture::Dragging;
                let i = press.item.unwrap_or_default();
                self.start_drag(ctx, i);
            }
            Gesture::Panning => self.scroll -= input.delta.y,
            _ => {}
        }
    }

    /// Right-clicking an unselected item selects just it; right-clicking empty space clears
    /// the selection.
    pub(super) fn right_click_select(&mut self, item: Option<usize>) {
        match item {
            Some(i) if !self.is_marked(i) => {
                self.marked.clear();
                self.selected = i;
            }
            Some(_) => {}
            None => self.marked.clear(),
        }
    }

    /// Handle a key press while this pane is focused. Returns true if it was used.
    pub fn key(&mut self, key: Key, mods: Modifiers) -> bool {
        let cols = self.cols.max(1) as isize;
        let page = self.page_items.max(1) as isize;
        match key {
            Key::ArrowUp if mods.alt => self.up(),
            Key::ArrowUp => self.move_selection(-cols),
            Key::ArrowDown => self.move_selection(cols),
            Key::ArrowLeft if self.view == View::Grid => self.move_selection(-1),
            Key::ArrowRight if self.view == View::Grid => self.move_selection(1),
            Key::PageUp => self.move_selection(-page),
            Key::PageDown => self.move_selection(page),
            Key::Home => self.move_selection(isize::MIN / 2),
            Key::End => self.move_selection(isize::MAX / 2),
            Key::Enter => self.activate(self.selected),
            Key::Backspace => self.up(),
            Key::Space => self.toggle_mark(self.selected),
            Key::Delete => self.request_delete(),
            Key::Escape => {
                if !self.marked.is_empty() || self.selecting {
                    self.clear_selection();
                } else {
                    set_clipboard(None);
                }
            }
            Key::A if mods.ctrl => self.select_all(),
            Key::G if mods.ctrl => self.toggle_view(),
            Key::H if mods.ctrl => self.toggle_hidden(),
            _ => return false,
        }
        true
    }

    /// Start dragging item `i` (with the rest of the selection, if it's part of it).
    pub(super) fn start_drag(&mut self, ctx: &egui::Context, i: usize) {
        if !self.is_marked(i) {
            self.marked.clear();
            self.selected = i;
        }
        let paths = self.targets();
        if !paths.is_empty() {
            crate::wl_dnd::log(format_args!("explorer drag started: {} path(s)", paths.len()));
            DragAndDrop::set_payload(ctx, ExplorerDrag { paths });
        }
    }

    /// A press released without moving: select, toggle or open.
    pub(super) fn click(&mut self, item: Option<usize>, touch: bool, mods: Modifiers, now: f64) {
        let Some(i) = item else {
            if !mods.ctrl && !mods.shift {
                self.marked.clear();
            }
            return;
        };
        if self.selecting {
            self.toggle_mark(i);
            if self.marked.is_empty() {
                self.selecting = false;
            }
        } else if touch {
            self.selected = i;
            self.activate(i);
        } else if mods.ctrl {
            if self.marked.is_empty() && i != self.selected {
                self.toggle_mark(self.selected);
            }
            self.toggle_mark(i);
        } else if mods.shift {
            let (a, b) = (self.selected.min(i), self.selected.max(i));
            self.marked = self.entries[a..=b].iter().map(|e| e.path.clone()).collect();
        } else {
            self.marked.clear();
            self.selected = i;
            match self.last_click {
                Some((last, at)) if last == i && now - at < DOUBLE_CLICK => {
                    self.last_click = None;
                    self.activate(i);
                }
                _ => self.last_click = Some((i, now)),
            }
        }
    }

    pub(super) fn apply(&mut self, ctx: &egui::Context, commands: Vec<Command>) {
        for cmd in commands {
            match cmd {
                Command::Open(i) => self.activate(i),
                Command::Copy => self.copy(ctx, false),
                Command::Cut => self.copy(ctx, true),
                Command::Paste => self.paste(ctx, None, None),
                Command::PasteInto(i) => {
                    let dest = self.entries.get(i).map(|e| e.path.clone());
                    self.paste(ctx, dest, None);
                }
                Command::ClearClipboard => set_clipboard(None),
                Command::CopyPath => {
                    let text = self.targets().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                    ctx.copy_text(text);
                }
                Command::Delete => self.request_delete(),
                Command::SelectMode => {
                    self.selecting = true;
                    if self.marked.is_empty() && self.targets().len() == 1 && !self.touch {
                        self.toggle_mark(self.selected);
                    }
                }
                Command::SelectAll => {
                    self.selecting = true;
                    self.select_all();
                }
                Command::ClearSelection => self.clear_selection(),
                Command::ToggleView => self.toggle_view(),
                Command::ToggleHidden => self.toggle_hidden(),
            }
        }
    }
}
