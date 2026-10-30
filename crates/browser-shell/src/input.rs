//! winit → Servo input translation for the web content area.

use servo::{
    Code, InputEvent, Key, KeyState, KeyboardEvent, Location, Modifiers, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent,
    NamedKey, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};
use servo::DevicePoint;
use winit::event::{ElementState, KeyEvent, MouseScrollDelta};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey as WNamed};

pub fn modifiers(m: ModifiersState) -> Modifiers {
    let mut r = Modifiers::empty();
    r.set(Modifiers::SHIFT, m.shift_key());
    r.set(Modifiers::CONTROL, m.control_key());
    r.set(Modifiers::ALT, m.alt_key());
    r.set(Modifiers::META, m.super_key());
    r
}

fn named(n: &WNamed) -> Option<NamedKey> {
    use NamedKey as N;
    Some(match n {
        WNamed::Enter => N::Enter,
        WNamed::Tab => N::Tab,
        WNamed::Backspace => N::Backspace,
        WNamed::Delete => N::Delete,
        WNamed::Escape => N::Escape,
        WNamed::ArrowLeft => N::ArrowLeft,
        WNamed::ArrowRight => N::ArrowRight,
        WNamed::ArrowUp => N::ArrowUp,
        WNamed::ArrowDown => N::ArrowDown,
        WNamed::Home => N::Home,
        WNamed::End => N::End,
        WNamed::PageUp => N::PageUp,
        WNamed::PageDown => N::PageDown,
        WNamed::Insert => N::Insert,
        WNamed::Shift => N::Shift,
        WNamed::Control => N::Control,
        WNamed::Alt => N::Alt,
        WNamed::Super => N::Meta,
        _ => return None,
    })
}

pub fn keyboard(ev: &KeyEvent, mods: ModifiersState) -> Option<InputEvent> {
    let key = match &ev.logical_key {
        WKey::Character(s) => Key::Character(s.to_string()),
        WKey::Named(WNamed::Space) => Key::Character(" ".into()),
        WKey::Named(n) => Key::Named(named(n)?),
        _ => return None,
    };
    let state = if ev.state == ElementState::Pressed { KeyState::Down } else { KeyState::Up };
    Some(InputEvent::Keyboard(KeyboardEvent::new({ let mut k = keyboard_types::KeyboardEvent::default(); k.state = state; k.key = key; k.code = Code::Unidentified; k.location = Location::Standard; k.modifiers = modifiers(mods); k.repeat = ev.repeat; k })))
}

pub fn mouse_move(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseMove(MouseMoveEvent::new(WebViewPoint::Device(DevicePoint::new(x, y))))
}

pub fn mouse_button(b: winit::event::MouseButton, st: ElementState, x: f32, y: f32) -> InputEvent {
    let button = match b {
        winit::event::MouseButton::Left => MouseButton::Primary,
        winit::event::MouseButton::Right => MouseButton::Secondary,
        winit::event::MouseButton::Middle => MouseButton::Auxiliary,
        winit::event::MouseButton::Back => MouseButton::Back,
        winit::event::MouseButton::Forward => MouseButton::Forward,
        winit::event::MouseButton::Other(n) => MouseButton::Other(n),
    };
    let action = if st == ElementState::Pressed { MouseButtonAction::Down } else { MouseButtonAction::Up };
    InputEvent::MouseButton(MouseButtonEvent::new(action, button, WebViewPoint::Device(DevicePoint::new(x, y))))
}

pub fn wheel(d: MouseScrollDelta, x: f32, y: f32) -> InputEvent {
    let (dx, dy, mode) = match d {
        MouseScrollDelta::LineDelta(dx, dy) => ((dx * 76.0) as f64, (dy * 76.0) as f64, WheelMode::DeltaLine),
        MouseScrollDelta::PixelDelta(p) => (p.x, p.y, WheelMode::DeltaPixel),
    };
    InputEvent::Wheel(WheelEvent::new(WheelDelta { x: dx, y: dy, z: 0.0, mode }, WebViewPoint::Device(DevicePoint::new(x, y))))
}

pub fn cursor_icon(c: servo::Cursor) -> winit::window::CursorIcon {
    use servo::Cursor as C;
    use winit::window::CursorIcon as W;
    match c {
        C::None | C::Default => W::Default,
        C::Pointer => W::Pointer,
        C::ContextMenu => W::ContextMenu,
        C::Help => W::Help,
        C::Progress => W::Progress,
        C::Wait => W::Wait,
        C::Cell => W::Cell,
        C::Crosshair => W::Crosshair,
        C::Text => W::Text,
        C::VerticalText => W::VerticalText,
        C::Alias => W::Alias,
        C::Copy => W::Copy,
        C::Move => W::Move,
        C::NoDrop => W::NoDrop,
        C::NotAllowed => W::NotAllowed,
        C::Grab => W::Grab,
        C::Grabbing => W::Grabbing,
        C::EResize => W::EResize,
        C::NResize => W::NResize,
        C::NeResize => W::NeResize,
        C::NwResize => W::NwResize,
        C::SResize => W::SResize,
        C::SeResize => W::SeResize,
        C::SwResize => W::SwResize,
        C::WResize => W::WResize,
        C::EwResize => W::EwResize,
        C::NsResize => W::NsResize,
        C::NeswResize => W::NeswResize,
        C::NwseResize => W::NwseResize,
        C::ColResize => W::ColResize,
        C::RowResize => W::RowResize,
        C::AllScroll => W::AllScroll,
        C::ZoomIn => W::ZoomIn,
        C::ZoomOut => W::ZoomOut,
    }
}
