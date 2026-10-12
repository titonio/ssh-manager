//! The dictionary between the two colour vocabularies this suite has to speak.
//!
//! `sshm` names a colour by a *role* (`accent`, `border`, `fg_muted`) and
//! `theme.rs` decides which token that role owns; the terminal reports back what
//! was actually painted, in `vt100`'s vocabulary of palette indices. A test that
//! compared those two had better have a translation, and the translation must
//! not be a place where a hue gets re-decided — so this module only renames.
//! `Theme::clack().accent` is the expectation; this says what the terminal calls
//! the colour that role was resolved to.
//!
//! The mapping is ratatui's own emission rule, not a preference: the named ANSI
//! colours become the base eight, their bright variants the upper eight, and an
//! indexed colour is the same index in both. `Reset` is the terminal's own ink,
//! which the emulator reports as its default.

/// What the terminal records for the colour a `Theme` role resolved to.
///
/// Takes the ratatui colour a role holds rather than a role name so this module
/// does not have to know what `Theme` contains — and so a test cannot quietly
/// compare against something other than the role it means to check.
pub fn token(role: ratatui::style::Color) -> vt100::Color {
    use ratatui::style::Color as C;
    match role {
        C::Reset => vt100::Color::Default,
        C::Black => vt100::Color::Idx(0),
        C::Red => vt100::Color::Idx(1),
        C::Green => vt100::Color::Idx(2),
        C::Yellow => vt100::Color::Idx(3),
        C::Blue => vt100::Color::Idx(4),
        C::Magenta => vt100::Color::Idx(5),
        C::Cyan => vt100::Color::Idx(6),
        C::Gray => vt100::Color::Idx(7),
        C::DarkGray => vt100::Color::Idx(8),
        C::LightRed => vt100::Color::Idx(9),
        C::LightGreen => vt100::Color::Idx(10),
        C::LightYellow => vt100::Color::Idx(11),
        C::LightBlue => vt100::Color::Idx(12),
        C::LightMagenta => vt100::Color::Idx(13),
        C::LightCyan => vt100::Color::Idx(14),
        C::White => vt100::Color::Idx(15),
        C::Indexed(index) => vt100::Color::Idx(index),
        C::Rgb(red, green, blue) => vt100::Color::Rgb(red, green, blue),
    }
}
