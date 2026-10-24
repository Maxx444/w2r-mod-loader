//! The host's own settings on the options screen (docs/settings-screen.md): each `host.json`
//! field's category, label and description, and reading and writing it in [`HostConfig`].
//!
//! `share_match_data` is in `sharing.json`, which the session handles. The lobby locks are not
//! here; the Create Multiplayer Game screen sets them.

use crate::hostcfg::{APM_ALERT_MAX, HostConfig};
use crate::settings::{Kind, Owner, SettingDef};

/// The key of the setting kept in `sharing.json`.
pub const SHARE_MATCH_DATA: &str = "share_match_data";

/// Every host setting the screen shows, in the order they are registered.
pub fn defs() -> Vec<SettingDef> {
    let host = |key: &str, kind: Kind, category: &str, sub: &str| {
        SettingDef::new(Owner::Host, key, kind, category, sub)
    };
    let defaults = HostConfig::default();
    vec![
        host("enhanced_create_screen", Kind::Bool, "UI", "Enhanced UIs")
            .label("Enhanced Create Multiplayer Game UI")
            .describe("The Create Multiplayer Game screen has a custom map selector, map preview, and restructured UI.")
            .default(i64::from(defaults.enhanced_create_screen)),
        host("enhanced_lobby_screen", Kind::Bool, "UI", "Enhanced UIs")
            .label("Enhanced Multiplayer Lobby UI")
            .describe("The multiplayer lobby has a map preview, compact game info, restructured UI, and additional host controls. A crown is shown beside the host and a green dot beside other players with the mod.")
            .default(i64::from(defaults.enhanced_lobby_screen)),
        host("hide_scrollbar_arrows", Kind::Bool, "UI", "Fixes")
            .label("Fix Overscroll")
            .describe("Fixes scrollable sections having empty space at the bottom. Requires disabling the scrollbar arrows.")
            .default(i64::from(defaults.hide_scrollbar_arrows)),
        host("apm_display", Kind::Bool, "UI", "APM Counter")
            .label("Display APM In Game")
            .describe("Shows your actions per minute on the game screen.")
            .default(i64::from(defaults.apm_display)),
        host("apm_alert_below", Kind::Int, "UI", "APM Counter")
            .label("Alert When APM Falls Below")
            .describe("Alerts you while your APM is below this number. 0 turns the alert off.")
            .range(0, i64::from(APM_ALERT_MAX))
            .default(i64::from(defaults.apm_alert_below))
            .active_when(Owner::Host, "apm_display"),
        host("apm_color_text", Kind::Bool, "UI", "APM Counter")
            .label("Color APM Text Red When Low")
            .describe("The APM turns red while it is below the alert number.")
            .default(i64::from(defaults.apm_color_text))
            .active_when(Owner::Host, "apm_display"),
        host("apm_play_sound", Kind::Bool, "UI", "APM Counter")
            .label("Play Sound When APM Is Low")
            .describe("A two-note chirp while your APM is below the alert number.")
            .default(i64::from(defaults.apm_play_sound))
            .active_when(Owner::Host, "apm_display"),
        host("auto_game_name", Kind::Bool, "Multiplayer", "Create Game")
            .label("Auto-Name Games by Map")
            .describe("Fills in the game name from the map while the name is empty, and again on each map change.")
            .default(i64::from(defaults.auto_game_name)),
        host("map_sharing", Kind::Bool, "Multiplayer", "Map Sharing")
            .label("Enable Map Sharing")
            .describe("Allows sending and receiving maps in multiplayer games to other players with the mod.")
            .default(i64::from(defaults.map_sharing)),
        host("auto_download", Kind::Bool, "Multiplayer", "Map Sharing")
            .label("Enable Auto Downloading")
            .describe("Automatically downloads missing maps when joining a multiplayer lobby hosted by a player with sharing enabled.")
            .default(i64::from(defaults.auto_download)),
        host("record_replays", Kind::Bool, "Multiplayer", "Replays")
            .label("Save Replays")
            .describe("Records each match you play as a replay.")
            .default(i64::from(defaults.record_replays)),
        host("minimap_ping", Kind::Bool, "Multiplayer", "In Game")
            .label("Minimap Pings (Alt+G, Click)")
            .describe("Alt+G then a click, or Ctrl+Alt+click, marks a spot on the minimap for your allies and observers.")
            .default(i64::from(defaults.minimap_ping)),
        host("net_panel", Kind::Bool, "Multiplayer", "In Game")
            .label("Show Connection Table In Game (Ctrl+Shift+N)")
            .describe("Starts each match with the connection table up: ping, jitter and loss for each player. Ctrl+Shift+N shows or hides it.")
            .default(i64::from(defaults.net_panel)),
        host(SHARE_MATCH_DATA, Kind::Bool, "Multiplayer", "Statistics")
            .label("Contribute Match Data")
            .describe("Uploads the results of your matches to the stats service, from the next match on."),
    ]
}

/// The keys of the host's bool settings, which a plugin's `active_when` may name as `host:<key>`.
pub fn bool_keys() -> Vec<&'static str> {
    vec![
        "enhanced_create_screen",
        "enhanced_lobby_screen",
        "hide_scrollbar_arrows",
        "apm_display",
        "apm_color_text",
        "apm_play_sound",
        "auto_game_name",
        "map_sharing",
        "auto_download",
        "record_replays",
        "minimap_ping",
        "net_panel",
        SHARE_MATCH_DATA,
    ]
}

/// A field of `cfg` by key (`share_match_data` is not in it).
pub fn read(cfg: &HostConfig, key: &str) -> Option<i64> {
    let b = |v: bool| Some(i64::from(v));
    match key {
        "enhanced_create_screen" => b(cfg.enhanced_create_screen),
        "enhanced_lobby_screen" => b(cfg.enhanced_lobby_screen),
        "hide_scrollbar_arrows" => b(cfg.hide_scrollbar_arrows),
        "apm_display" => b(cfg.apm_display),
        "apm_alert_below" => Some(i64::from(cfg.apm_alert_below)),
        "apm_color_text" => b(cfg.apm_color_text),
        "apm_play_sound" => b(cfg.apm_play_sound),
        "auto_game_name" => b(cfg.auto_game_name),
        "map_sharing" => b(cfg.map_sharing),
        "auto_download" => b(cfg.auto_download),
        "record_replays" => b(cfg.record_replays),
        "minimap_ping" => b(cfg.minimap_ping),
        "net_panel" => b(cfg.net_panel),
        _ => None,
    }
}

/// Sets a field of `cfg` by key. False for a key that is not a `host.json` field.
pub fn write(cfg: &mut HostConfig, key: &str, value: i64) -> bool {
    let on = value != 0;
    match key {
        "enhanced_create_screen" => cfg.enhanced_create_screen = on,
        "enhanced_lobby_screen" => cfg.enhanced_lobby_screen = on,
        "hide_scrollbar_arrows" => cfg.hide_scrollbar_arrows = on,
        "apm_display" => cfg.apm_display = on,
        "apm_alert_below" => {
            cfg.apm_alert_below = value.clamp(0, i64::from(APM_ALERT_MAX)) as u32;
        }
        "apm_color_text" => cfg.apm_color_text = on,
        "apm_play_sound" => cfg.apm_play_sound = on,
        "auto_game_name" => cfg.auto_game_name = on,
        "map_sharing" => cfg.map_sharing = on,
        "auto_download" => cfg.auto_download = on,
        "record_replays" => cfg.record_replays = on,
        "minimap_ping" => cfg.minimap_ping = on,
        "net_panel" => cfg.net_panel = on,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_def_reads_and_writes_or_is_the_sharing_flag() {
        let mut cfg = HostConfig::default();
        for d in defs() {
            if d.key == SHARE_MATCH_DATA {
                assert_eq!(read(&cfg, &d.key), None);
                continue;
            }
            let v = read(&cfg, &d.key).unwrap_or_else(|| panic!("{} unread", d.key));
            assert_eq!(v, d.default_int, "{} default", d.key);
            let next = if d.kind == Kind::Bool { 1 - v } else { 5 };
            assert!(write(&mut cfg, &d.key, next), "{}", d.key);
            assert_eq!(read(&cfg, &d.key), Some(next), "{}", d.key);
        }
        assert!(!write(&mut cfg, "nope", 1));
    }

    #[test]
    fn bool_keys_are_exactly_the_bool_defs() {
        let mut from_defs: Vec<String> = defs()
            .into_iter()
            .filter(|d| d.kind == Kind::Bool)
            .map(|d| d.key)
            .collect();
        let mut listed: Vec<String> = bool_keys().into_iter().map(str::to_owned).collect();
        from_defs.sort();
        listed.sort();
        assert_eq!(from_defs, listed);
    }

    #[test]
    fn labels_are_title_case_ascii() {
        for d in defs() {
            assert!(d.label.is_ascii() && d.description.is_ascii(), "{}", d.key);
            assert!(
                d.label
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
            );
        }
    }

    #[test]
    fn the_alert_is_clamped() {
        let mut cfg = HostConfig::default();
        write(&mut cfg, "apm_alert_below", 5000);
        assert_eq!(cfg.apm_alert_below, APM_ALERT_MAX);
    }
}
