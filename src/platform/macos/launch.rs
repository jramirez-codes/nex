//! Starting from nex.app. Finder and the Dock start apps in `/` with a bare environment, so
//! nex fills in what a shell started from Terminal would have had.

use std::sync::OnceLock;

/// Started from nex.app (Finder, the Dock, Spotlight), not from a shell: the binary is
/// inside a bundle and the working directory is `/`.
pub fn opened_from_desktop() -> bool {
    let in_bundle = std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"));
    in_bundle && std::env::current_dir().is_ok_and(|d| d == std::path::Path::new("/"))
}

/// `LANG` for the shell when nex didn't inherit one (apps opened from Finder don't), from
/// the system's region setting, so UTF-8 works in the shell and its programs.
pub fn shell_env() -> Vec<(String, String)> {
    if std::env::var_os("LANG").is_some_and(|l| !l.is_empty()) {
        return Vec::new();
    }
    vec![("LANG".to_string(), system_lang().to_string())]
}

fn system_lang() -> &'static str {
    static LANG: OnceLock<String> = OnceLock::new();
    LANG.get_or_init(|| {
        let locale = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        lang_from_locale(&locale)
    })
}

/// `en_US` or `en_US@rg=gbzzzz` → `en_US.UTF-8`; anything unusable → `en_US.UTF-8`.
fn lang_from_locale(locale: &str) -> String {
    let base = locale.split('@').next().unwrap_or_default();
    let valid = matches!(base.split_once('_'), Some((lang, region))
        if (2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_lowercase())
            && region.len() == 2 && region.chars().all(|c| c.is_ascii_uppercase()));
    if valid { format!("{base}.UTF-8") } else { "en_US.UTF-8".to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_to_lang() {
        assert_eq!(lang_from_locale("en_US"), "en_US.UTF-8");
        assert_eq!(lang_from_locale("de_DE@rg=chzzzz"), "de_DE.UTF-8");
        assert_eq!(lang_from_locale("en"), "en_US.UTF-8");
        assert_eq!(lang_from_locale(""), "en_US.UTF-8");
        assert_eq!(lang_from_locale("zh-Hans_CN"), "en_US.UTF-8");
    }
}
