//! Interface language: Russian or English. Texts are written in place as `tr!("рус", "eng")`.

use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

/// A text in both languages: (Russian, English).
pub type Text = (&'static str, &'static str);

/// The interface is in English.
pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// The text in the current language.
pub fn pick(t: Text) -> &'static str {
    if en() {
        t.1
    } else {
        t.0
    }
}

/// Sets the language: "ru", "en", or follow Windows when not chosen.
pub fn set(lang: Option<&str>) {
    // RGBTRAY_LANG=en|ru overrides the setting (for screenshots and testing).
    let forced = std::env::var("RGBTRAY_LANG").ok();
    let en = english(forced.as_deref(), lang).unwrap_or_else(|| !windows_is_russian());
    EN.store(en, Ordering::Relaxed);
}

/// Whether to use English: the forced language, else the saved one; None when neither is "ru"
/// or "en" (any case), so an unknown value doesn't hide a valid choice behind it.
fn english(forced: Option<&str>, saved: Option<&str>) -> Option<bool> {
    let parse = |l: &str| match l.trim().to_ascii_lowercase().as_str() {
        "ru" => Some(false),
        "en" => Some(true),
        _ => None,
    };
    forced.and_then(parse).or_else(|| saved.and_then(parse))
}

/// The Windows display language, or one of the keyboard layouts, is Russian or a language
/// whose speakers usually read Russian. Many Russian speakers run an English Windows with a
/// Russian keyboard layout.
fn windows_is_russian() -> bool {
    // Primary language ids: Russian, Ukrainian, Belarusian, Kazakh, Kyrgyz, Uzbek, Tajik.
    let russian = |lang: u16| matches!(lang & 0x3FF, 0x19 | 0x22 | 0x23 | 0x3F | 0x40 | 0x43 | 0x28);
    unsafe {
        if russian(windows_sys::Win32::Globalization::GetUserDefaultUILanguage()) {
            return true;
        }
        let mut layouts = [std::ptr::null_mut(); 32];
        let n = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList(layouts.len() as i32, layouts.as_mut_ptr());
        layouts[..n.max(0) as usize].iter().any(|&hkl| russian(hkl as usize as u16))
    }
}

/// `tr!("рус", "eng")`: the text in the current language.
#[macro_export]
macro_rules! tr {
    ($ru:expr, $en:expr) => {
        if $crate::i18n::en() {
            $en
        } else {
            $ru
        }
    };
}

#[cfg(test)]
mod tests {
    use super::english;

    #[test]
    fn forced_language_wins_only_when_valid() {
        assert_eq!(english(Some("en"), Some("ru")), Some(true));
        assert_eq!(english(Some("RU"), Some("en")), Some(false));
        // An unknown forced value falls back to the saved language.
        assert_eq!(english(Some("de"), Some("ru")), Some(false));
        assert_eq!(english(Some(""), Some("en")), Some(true));
        assert_eq!(english(None, Some("en")), Some(true));
        // Neither is valid: follow Windows.
        assert_eq!(english(Some("fr"), Some("xx")), None);
        assert_eq!(english(None, None), None);
    }
}
