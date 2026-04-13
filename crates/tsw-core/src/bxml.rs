//! Write pre-compiled .bxml cache files after install.
//!
//! The game generates these at runtime on first load, but the first-time
//! compilation can crash when connecting with an existing character.
//! We embed them in the binary and write them post-install to avoid the issue.

use std::path::Path;

/// Shader cache files — GPU capability profiles that the game's shader compiler
/// reads to determine which shader features to enable (e.g., ATI2N normal map
/// compression, depth texture format, MSAA settings). Without these, the game
/// uses fallback shader paths that render textures incorrectly (cross-hatch
/// artifacts on normal maps). The game normally generates these on first launch,
/// but fails to do so without the game patcher's initialization sequence.
const SHADER_CACHE_FILES: &[(&str, &[u8])] = &[
    ("shader.cache.local3", include_bytes!("shader_cache/shader.cache.local3")),
    ("shader.cache.dx11.local3", include_bytes!("shader_cache/shader.cache.dx11.local3")),
];

/// Each entry: (relative path from install dir, file bytes)
const BXML_FILES: &[(&str, &[u8])] = &[
    // Bundle group definitions (controls texture bundle loading)
    ("Data/Gui/Default/BundleGroups.bxml", include_bytes!("bxml_cache/Data/Gui/Default/BundleGroups.bxml")),
    // Top-level config caches
    ("Data/Gui/Default/FontConfig.bxml", include_bytes!("bxml_cache/Data/Gui/Default/FontConfig.bxml")),
    ("Data/Gui/Default/Fonts.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Fonts.bxml")),
    ("Data/Gui/Default/Modules.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Modules.bxml")),
    ("Data/Gui/Default/TextColors.bxml", include_bytes!("bxml_cache/Data/Gui/Default/TextColors.bxml")),
    ("Data/Gui/Default/Variables.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Variables.bxml")),
    // Banner config
    ("Data/Gui/Default/Flash/Banners/BannerConfig.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Flash/Banners/BannerConfig.bxml")),
    // View templates
    ("Data/Gui/Default/ViewTemplates/GUITKClasses.bxml", include_bytes!("bxml_cache/Data/Gui/Default/ViewTemplates/GUITKClasses.bxml")),
    ("Data/Gui/Default/ViewTemplates/GameClasses.bxml", include_bytes!("bxml_cache/Data/Gui/Default/ViewTemplates/GameClasses.bxml")),
    ("Data/Gui/Default/ViewTemplates/LargeClasses.bxml", include_bytes!("bxml_cache/Data/Gui/Default/ViewTemplates/LargeClasses.bxml")),
    ("Data/Gui/Default/ViewTemplates/TeamGUIClasses.bxml", include_bytes!("bxml_cache/Data/Gui/Default/ViewTemplates/TeamGUIClasses.bxml")),
    // Views
    ("Data/Gui/Default/Views/Chat/ChatWindowSkin.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/Chat/ChatWindowSkin.bxml")),
    ("Data/Gui/Default/Views/DebugCenter/BugReportBtnView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/DebugCenter/BugReportBtnView.bxml")),
    ("Data/Gui/Default/Views/DrowningBar.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/DrowningBar.bxml")),
    ("Data/Gui/Default/Views/HUD/HUDView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/HUD/HUDView.bxml")),
    ("Data/Gui/Default/Views/HUDMapView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/HUDMapView.bxml")),
    ("Data/Gui/Default/Views/MainMenu/MainMenuView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MainMenu/MainMenuView.bxml")),
    ("Data/Gui/Default/Views/MainMenu/Options.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MainMenu/Options.bxml")),
    ("Data/Gui/Default/Views/MainMenu/OptionsView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MainMenu/OptionsView.bxml")),
    ("Data/Gui/Default/Views/MainMenu/Window.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MainMenu/Window.bxml")),
    ("Data/Gui/Default/Views/MapGUI/RegionMapRenderer.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MapGUI/RegionMapRenderer.bxml")),
    ("Data/Gui/Default/Views/MapGUI/WorldMapRegionPositions.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MapGUI/WorldMapRegionPositions.bxml")),
    ("Data/Gui/Default/Views/MemoryManagerDebug/FragmentationView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MemoryManagerDebug/FragmentationView.bxml")),
    ("Data/Gui/Default/Views/MemoryManagerDebug/PoolView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/MemoryManagerDebug/PoolView.bxml")),
    ("Data/Gui/Default/Views/PortraitGUI/OverheadConfig.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/PortraitGUI/OverheadConfig.bxml")),
    ("Data/Gui/Default/Views/SplashScreenView.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/SplashScreenView.bxml")),
    ("Data/Gui/Default/Views/TokenGUI/TokenIcons.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Views/TokenGUI/TokenIcons.bxml")),
    // Waypoints (playfield data — crash if missing when entering these zones)
    ("Data/Gui/Default/Waypoints/PF1100.bxml", include_bytes!("bxml_cache/Data/Gui/Default/Waypoints/PF1100.bxml")),
    // Window skins
    ("Data/Gui/Default/WindowSkins/Borderless.bxml", include_bytes!("bxml_cache/Data/Gui/Default/WindowSkins/Borderless.bxml")),
    ("Data/Gui/Default/WindowSkins/Tabbed.bxml", include_bytes!("bxml_cache/Data/Gui/Default/WindowSkins/Tabbed.bxml")),
];

/// Write all cached bxml files and shader cache files to the install directory.
/// Skips files that already exist (doesn't overwrite user-generated cache).
pub fn write_bxml_cache(install_dir: &Path) -> Result<u32, String> {
    let mut written = 0u32;
    for (rel_path, data) in BXML_FILES {
        let dest = install_dir.join(rel_path);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir for {}: {}", rel_path, e))?;
        }
        std::fs::write(&dest, data)
            .map_err(|e| format!("Failed to write {}: {}", rel_path, e))?;
        written += 1;
    }

    // Write shader cache files (GPU capability profiles)
    for (rel_path, data) in SHADER_CACHE_FILES {
        let dest = install_dir.join(rel_path);
        if dest.exists() {
            continue;
        }
        std::fs::write(&dest, data)
            .map_err(|e| format!("Failed to write {}: {}", rel_path, e))?;
        written += 1;
    }

    Ok(written)
}
