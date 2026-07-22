//! Launch-time fidelity settings shared by native automation clients.

/// CVars owned by the automation substrate for every native game process.
///
/// Launchers append these values after caller-supplied arguments so persistent
/// user settings cannot change replay behavior. In particular, the TV
/// calibration screen remains visible, matching the console flow, without
/// changing Dusklight's normal defaults or gameplay source.
pub const FIXED_AUTOMATION_CVARS: [&str; 5] = [
    "game.instantSaves=true",
    "backend.cardFileType=1",
    "backend.wasPresetChosen=true",
    "game.enableMenuPointer=false",
    "game.hideTvSettingsScreen=false",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_automation_cvars_preserve_the_console_tv_screen() {
        assert_eq!(FIXED_AUTOMATION_CVARS.len(), 5);
        assert_eq!(
            FIXED_AUTOMATION_CVARS.last(),
            Some(&"game.hideTvSettingsScreen=false")
        );
        assert!(!FIXED_AUTOMATION_CVARS.contains(&"game.hideTvSettingsScreen=true"));
    }
}
